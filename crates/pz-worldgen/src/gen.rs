//! `zombie.iso.worldgen.WorldGenChunk` and `WorldGenTile`: world
//! generation over a 2×2 block of brand-new chunks.
//!
//! Map chunks (`genMapChunk`) keep the lotpack's squares; each square's
//! biome comes from the cell's biome-map image and its vegetation
//! placeholders (lotpack trees, bushes and grass) are replaced with biome
//! features, possibly multi-square (jumbo trees, boulder groups), with ore
//! boulders added on bare ground. Chunks with empty squares at level 0
//! (`genRandomChunk`, the procedural area around the map) get generated
//! ground, vegetation, roads and ore veins from noise-selected biomes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_world::cell_loader::{do_tile_object_creation, ObjectEnv};
use pz_world::chunk::ChunksCache;
use pz_world::chunk::{ChunkGenerationStatus, ChunkLoadEnv, IsoChunk, IsoGridSquare};
use pz_world::objects::{IsoObject, ObjectId, ObjectKind};
use pz_world::sprite::SpriteIdx;
use pz_world::{IsoMetaGrid, IsoSpriteManager};

use crate::biome_map::{BiomeBand, BiomeMap};
use crate::defs::{
    Biome, Feature, FeatureType, PrefabStructure, Selection, StaticModule, TileGroup, AXES, BUSH,
    HYGROMETRY, LANDSCAPE, MISSING_TILE, ORE_LEVEL, PLANT, TEMPERATURE,
};
use crate::java::{npe, wildcard_regex, JResult, JavaException, JavaRandom, WildcardCache};
use crate::params::{WorldGenParams, GENERATION_SIZE, GENERATION_SQUARES};
use crate::roads::{Road, RoadGenerator};
use crate::simplex::WorldGenSimplexGenerator;
use crate::veins::Veins;
use crate::zones::WorldGenZones;

const ANY: &str = "$any";
const SUBBIOME: &str = "$subbiome";
const NO_TREE: &str = "$no_tree";
const NO_BUSH: &str = "$no_bush";
const NO_GRASS: &str = "$no_grass";

const BLOCK: usize = (GENERATION_SQUARES * GENERATION_SQUARES) as usize;

/// `TileReplacement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileReplacement {
    Success,
    Failure,
    Pending,
    Delete,
}

/// `TileLookup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TileLookup {
    NoFeatures,
    NoSpecificType,
    FeaturesListEmpty,
    NoFeature,
    TileGroupsEmpty,
    TooBig,
    CantBePlaced,
    AlreadyPlaced,
    Success,
}

/// The `toBeDone` grid: per feature type, a tile (or `$` marker) waiting
/// for each square of the 16 × 16 block.
type ToBeDone = Vec<Vec<Option<String>>>;

/// `BlendDirection` indices (`NORTH`, `SOUTH`, `WEST`, `EAST`).
const NORTH: usize = 0;
const SOUTH: usize = 1;
const WEST: usize = 2;
const EAST: usize = 3;

/// A square position `(x, y, z)`.
pub type SquarePos = (i32, i32, i32);

/// Everything `WorldGenChunk` reads, built once per world.
pub struct WorldGen {
    pub params: WorldGenParams,
    pub simplex: WorldGenSimplexGenerator,
    /// `biomes`: generating biomes for the procedural area, in `HashMap`
    /// order.
    pub biomes: Vec<Rc<Biome>>,
    /// `biomesMap`: generating biomes the biome map names, in `HashMap`
    /// order.
    pub biomes_map: Vec<Rc<Biome>>,
    /// `landscape`, `plant`, `bush`, `temperature`, `hygrometry`,
    /// `oreLevel` selections, in [`AXES`] order.
    pub selection: [Selection; 6],
    pub static_modules: Vec<StaticModule>,
    pub veins: Veins,
    pub priorities: HashMap<String, f64>,
    pub road_generators: Vec<RoadGenerator>,
    pub biome_map: BiomeMap,
    pub zones: WorldGenZones,
    /// `IsoCell.getWorldX()/getWorldY()`, the origin `applyOreVeins`
    /// searches veins around (0 unless the cell was moved).
    pub cell_world: (i32, i32),
    /// `IsoChunk.proceduralZombieSquares` of generated chunks: squares
    /// that get a zombie when the chunk is added to the world.
    pub procedural_zombie_squares: HashMap<(i32, i32), Vec<SquarePos>>,
    /// Exceptions generation hit (the original would fail the chunk load).
    pub errors: Vec<String>,
    default_biome: Rc<Biome>,
    wild: WildcardCache,
    biome_cache: RefCell<HashMap<[u8; 6], Vec<usize>>>,
    map_biome_cache: RefCell<HashMap<(String, u8, u8), Vec<usize>>>,
}

/// `IsoObject.isBush()` etc. on a sprite.
struct Kinds<'m> {
    m: &'m IsoSpriteManager,
}

impl Kinds<'_> {
    fn name<'a>(&'a self, o: &IsoObject) -> Option<&'a str> {
        o.sprite.and_then(|s| self.m.sprites[s].name.as_deref())
    }
    fn tree(&self, o: &IsoObject) -> bool {
        matches!(o.kind, ObjectKind::Tree(_))
    }
    fn bush(&self, o: &IsoObject) -> bool {
        let Some(s) = o.sprite else { return false };
        let s = &self.m.sprites[s];
        s.tileset_name.as_deref() == Some("f_bushes_1")
            || s.properties.get(&self.m.alias, "Bush").is_some()
    }
    fn grass(&self, o: &IsoObject) -> bool {
        self.name(o)
            .is_some_and(|n| n.starts_with("e_newgrass_") || n.starts_with("blends_grassoverlays_"))
    }
    fn grass_like(&self, o: &IsoObject) -> bool {
        self.grass(o)
            || self.name(o).is_some_and(|n| {
                n.starts_with("d_plants_")
                    || n.starts_with("d_generic_1_")
                    || n.starts_with("d_floorleaves_")
                    || n.starts_with("vegetation_groundcover_")
            })
    }
    fn ores(&self, o: &IsoObject) -> bool {
        self.name(o)
            .is_some_and(|n| n.starts_with("boulders_") || n.starts_with("crafting_ore_"))
    }
    fn first(&self, sq: &IsoGridSquare, f: impl Fn(&Self, &IsoObject) -> bool) -> Option<ObjectId> {
        sq.objects
            .iter()
            .map(|o| o.borrow())
            .find(|o| f(self, o))
            .map(|o| o.id)
    }
    fn count(&self, sq: &IsoGridSquare, f: impl Fn(&Self, &IsoObject) -> bool) -> usize {
        sq.objects.iter().filter(|o| f(self, &o.borrow())).count()
    }
    /// `getTree()`, `getBush()`, `getGrass()` for TREE, BUSH, PLANT.
    fn current(&self, sq: &IsoGridSquare, ty: FeatureType) -> Option<ObjectId> {
        match ty {
            FeatureType::Tree => self.first(sq, Self::tree),
            FeatureType::Bush => self.first(sq, Self::bush),
            FeatureType::Plant => self.first(sq, Self::grass),
            _ => None,
        }
    }
}

/// `IsoGridSquare.DeleteTileObject(obj)` (by identity; a deleted object's
/// sprite is cleared, so later lookups through it see no sprite).
fn delete(sq: &mut IsoGridSquare, id: Option<ObjectId>) {
    if let Some(id) = id {
        if let Some(i) = sq.objects.iter().position(|o| o.borrow().id == id) {
            sq.remove_object_slot(i);
        }
    }
}

/// Position of the square being generated.
#[derive(Clone, Copy, Debug)]
struct At {
    /// The chunk (`ch.wx`, `ch.wy`).
    ch: (i32, i32),
    /// World square.
    x: i32,
    y: i32,
    /// Square within the 16 × 16 block (`ccTileX/Y`).
    cc_x: i32,
    cc_y: i32,
}

impl At {
    fn local(&self) -> (i32, i32) {
        (self.x - self.ch.0 * 8, self.y - self.ch.1 * 8)
    }
    fn tbd(&self) -> usize {
        (self.cc_y * GENERATION_SQUARES + self.cc_x) as usize
    }
}

fn square_mut(chunks: &mut ChunksCache, at: &At) -> Option<pz_world::SquareRef> {
    let (lx, ly) = at.local();
    chunks
        .get_mut(at.ch.0, at.ch.1)?
        .get_grid_square_mut(lx, ly, 0)
}

fn square(chunks: &ChunksCache, at: &At) -> Option<pz_world::SquareRef> {
    let (lx, ly) = at.local();
    chunks.get(at.ch.0, at.ch.1)?.get_grid_square(lx, ly, 0)
}

fn chunk_mut<'a>(chunks: &'a mut ChunksCache, at: &At) -> &'a mut IsoChunk {
    chunks.get_mut(at.ch.0, at.ch.1).expect("generated chunk")
}

/// `IsoGridSquare.getNew(cell, null, x, y, 0)` put in its chunk.
fn new_square(meta: &IsoMetaGrid, x: i32, y: i32) -> JResult<IsoGridSquare> {
    let mut sq = IsoGridSquare::new(x, y, 0);
    sq.associated_building = meta
        .get_associated_building_at(x, y)
        .map_err(JavaException)?;
    Ok(sq)
}

/// `AttachmentsHandler.resetAttachments(chunk)`.
fn reset_attachments(ch: &mut IsoChunk) {
    ch.saved.attachments_done_full = false;
    ch.saved.attachments_state = [false; 5];
}

/// `findFeature(features, prefilter, postfilter, rnd)`.
fn find_feature<'a>(
    features: &[&'a Feature],
    pre: f32,
    post: f32,
    rnd: &mut JavaRandom,
) -> Option<&'a Feature> {
    if features.is_empty() {
        return None;
    }
    let r = rnd.next_float();
    let mut p = 0.0f32;
    for f in features {
        p += f.probability / post * pre;
        if r >= p {
            continue;
        }
        return Some(f);
    }
    None
}

/// Per-square inputs `WorldGenChunk` threads through its helpers.
struct Ctx<'c, 'e, 'a> {
    chunks: &'c mut ChunksCache,
    meta: &'c IsoMetaGrid,
    env: &'c mut ChunkLoadEnv<'e>,
    to_be_done: &'a mut ToBeDone,
}

impl WorldGen {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        params: WorldGenParams,
        simplex: WorldGenSimplexGenerator,
        biomes: Vec<Rc<Biome>>,
        biomes_map: Vec<Rc<Biome>>,
        selection: [Selection; 6],
        static_modules: Vec<StaticModule>,
        veins: Veins,
        priorities: HashMap<String, f64>,
        road_generators: Vec<RoadGenerator>,
        biome_map: BiomeMap,
        zones: WorldGenZones,
    ) -> Self {
        WorldGen {
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
            cell_world: (0, 0),
            procedural_zombie_squares: HashMap::new(),
            errors: Vec::new(),
            default_biome: Rc::new(Biome::default_biome()),
            wild: WildcardCache::default(),
            biome_cache: RefCell::new(HashMap::new()),
            map_biome_cache: RefCell::new(HashMap::new()),
        }
    }

    /// `generateChunks(chunks)`: each chunk of the block, x outer and y
    /// inner. A chunk with an empty level-0 square is generated from
    /// scratch; otherwise its map squares are reworked and sand / road
    /// floors cleared of ground cover.
    pub fn generate_chunks(
        &mut self,
        chunks: &mut ChunksCache,
        meta: &IsoMetaGrid,
        env: &mut ChunkLoadEnv<'_>,
    ) -> JResult<()> {
        let mut tbd: ToBeDone = vec![vec![None; BLOCK]; FeatureType::ALL.len()];
        let (min_x, min_y) = (chunks.min_chunk_x(), chunks.min_chunk_y());
        for x in min_x..min_x + GENERATION_SIZE {
            for y in min_y..min_y + GENERATION_SIZE {
                let ch = chunks.get(x, y).ok_or_else(|| npe("chunk of the block"))?;
                let empty = has_empty_squares_on_level_zero(ch);
                let mut ctx = Ctx {
                    chunks: &mut *chunks,
                    meta,
                    env: &mut *env,
                    to_be_done: &mut tbd,
                };
                if empty {
                    self.gen_random_chunk(&mut ctx, (x, y))?;
                    continue;
                }
                self.gen_map_chunk(&mut ctx, (x, y))?;
                let ch = chunks.get_mut(x, y).expect("chunk");
                self.clean_chunk(ch, env.sprites, "Sand", "vegetation_groundcover_01");
                self.clean_chunk(ch, env.sprites, "Road_*", "vegetation_groundcover_01");
            }
        }
        Ok(())
    }

    fn at(&self, chunks: &ChunksCache, ch: (i32, i32), x: i32, y: i32) -> At {
        At {
            ch,
            x,
            y,
            cc_x: x - chunks.min_tile_x(),
            cc_y: y - chunks.min_tile_y(),
        }
    }

    /// `genRandomChunk`.
    fn gen_random_chunk(&mut self, c: &mut Ctx<'_, '_, '_>, key: (i32, i32)) -> JResult<()> {
        start_random_chunk(c.chunks.get_mut(key.0, key.1).expect("chunk"));
        let mut roads: Vec<Road> = Vec::new();
        for g in &self.road_generators {
            roads = g.roads(key.0, key.1);
        }
        let (min_x, min_y) = (key.0 * 8, key.1 * 8);
        let mut result = Ok(());
        'outer: for x in min_x..min_x + 8 {
            for y in min_y..min_y + 8 {
                let at = self.at(c.chunks, key, x, y);
                if let Err(e) = self.gen_random_square(c, at, &roads) {
                    result = Err(e);
                    break 'outer;
                }
            }
        }
        if let Err(e) = result {
            // "Failed to load chunk, blocking out area".
            self.errors.push(format!("chunk {},{}: {e}", key.0, key.1));
            let ch = c.chunks.get_mut(key.0, key.1).expect("chunk");
            for x in 0..8 {
                for y in 0..8 {
                    for z in 0..ch.max_level + 1 {
                        ch.set_square(x, y, z, None);
                    }
                }
            }
        }
        Ok(())
    }

    /// `genRandomSquare`.
    fn gen_random_square(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: At,
        roads: &[Road],
    ) -> JResult<()> {
        let (tx, ty) = at.local();
        let ch = chunk_mut(c.chunks, &at);
        if let Some(sq_owner) = ch.get_grid_square(tx, ty, 0) {
            let sq = sq_owner.borrow();
            if !sq.objects.is_empty() {
                let s = &mut ch.saved;
                s.blending_done_full = false;
                s.blending_done_partial = true;
                s.blending_depth[NORTH] = (ty as i8).min(s.blending_depth[NORTH]);
                s.blending_depth[SOUTH] = (ty as i8).max(s.blending_depth[SOUTH]);
                s.blending_depth[WEST] = (tx as i8).min(s.blending_depth[WEST]);
                s.blending_depth[EAST] = (tx as i8).max(s.blending_depth[EAST]);
                return Ok(());
            }
        } else {
            let sq = new_square(c.meta, at.x, at.y)?;
            ch.set_square(tx, ty, 0, Some((sq).into()));
        }
        let sq_owner = ch.get_grid_square_mut(tx, ty, 0).expect("square");
        let mut sq = sq_owner.borrow_mut();
        sq.room_id = -1;
        sq.reset_iso_world_region();
        drop(sq);
        let (x, y) = (at.x, at.y);
        let module = self
            .static_modules
            .iter()
            .position(|m| x >= m.xmin && x <= m.xmax && y >= m.ymin && y <= m.ymax);
        let seed_x = at.ch.0.wrapping_mul(8).wrapping_add(x);
        let seed_y = at.ch.1.wrapping_mul(8).wrapping_add(y);
        let mut rnd = self.params.random(seed_x, seed_y, 0);
        match module {
            None => {
                let mut placed = false;
                let mut rnd_road = self.params.random(seed_x, seed_y, 0);
                for road in roads {
                    if x < road.a.0.min(road.b.0)
                        || x > road.a.0.max(road.b.0)
                        || y < road.a.1.min(road.b.1)
                        || y > road.a.1.max(road.b.1)
                        || rnd_road.next_double()
                            >= self.road_generators[road.generator].config.proba_roads
                    {
                        continue;
                    }
                    let tiles = self.road_generators[road.generator].tiles();
                    let tg = &tiles[rnd_road.next_int_bound(tiles.len() as i32) as usize];
                    let tile = tg.tiles[0].clone();
                    self.apply_tile(c, &at, &tile, &mut rnd_road)?;
                    placed = true;
                }
                if !placed {
                    let biome = self.get_biome(x, y)?;
                    self.gen_random_tiles(c, at, biome, &mut rnd)?;
                }
            }
            Some(i) => {
                let m = &self.static_modules[i];
                if let Some(b) = m.biome.clone() {
                    self.gen_random_tiles(c, at, b, &mut rnd)?;
                } else if let Some(p) = m.prefab.clone() {
                    let biome = self.get_biome(x, y)?;
                    let (xmin, ymin) = (m.xmin, m.ymin);
                    self.apply_prefab(c, at, &p, biome, xmin, ymin, &mut rnd)?;
                } else {
                    return Err(JavaException(
                        "static module without biome or prefab".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// `genRandomTiles`.
    fn gen_random_tiles(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: At,
        biome: Rc<Biome>,
        rnd: &mut JavaRandom,
    ) -> JResult<()> {
        let mut subs: HashMap<FeatureType, Rc<Biome>> = HashMap::new();
        let mut biome = biome;
        self.do_pending(c, &at, Some(&biome), Some(&biome), &mut subs)?;
        for ty in FeatureType::ALL {
            if let Some(b) = subs.get(&ty) {
                biome = b.clone();
            }
            let r = self.apply_biome(c, &at, &biome, ty, rnd)?;
            if r.0 != TileReplacement::Pending {
                continue;
            }
            self.do_pending(c, &at, Some(&biome), Some(&biome), &mut subs)?;
            if let Some(b) = subs.get(&ty) {
                biome = b.clone();
            }
            self.apply_biome(c, &at, &biome, ty, rnd)?;
        }
        self.apply_ore_veins(c, &at, rnd)
    }

    /// `genMapChunk`.
    fn gen_map_chunk(&mut self, c: &mut Ctx<'_, '_, '_>, key: (i32, i32)) -> JResult<()> {
        let Some(biomes) = self.biome_map.zones(key.0, key.1, BiomeBand::Biome) else {
            return Ok(());
        };
        for x in key.0 * 8..key.0 * 8 + 8 {
            for y in key.1 * 8..key.1 * 8 + 8 {
                let at = self.at(c.chunks, key, x, y);
                self.gen_map_square(c, at, &biomes)?;
            }
        }
        Ok(())
    }

    /// `genMapSquare`.
    fn gen_map_square(&mut self, c: &mut Ctx<'_, '_, '_>, at: At, biomes: &[i32]) -> JResult<()> {
        let (tx, ty) = at.local();
        let (x, y) = (at.x, at.y);
        self.replace_square(c, at, biomes)?;
        let rocks = self.zones.zone_at(x, y, 0).is_none_or(|z| z.rocks);
        let m = &*c.env.sprites;
        let Some(sq_owner) = square(c.chunks, &at) else {
            return Ok(());
        };
        let sq = sq_owner.borrow();
        let Some(fi) = sq.floor_index(m) else {
            return Ok(());
        };
        let floor_id = sq.objects[fi].borrow().id;
        drop(sq);
        let Some(lookup) = self
            .biome_map
            .entry(biomes[(ty * 8 + tx) as usize])
            .cloned()
        else {
            return Ok(());
        };
        if lookup.biome.as_deref() == Some("$random") {
            let ch = chunk_mut(c.chunks, &at);
            reset_attachments(ch);
            let sq = new_square(c.meta, x, y)?;
            ch.set_square(tx, ty, 0, Some((sq).into()));
            return self.gen_random_square(c, at, &[]);
        }
        let mut rnd = self.params.random(
            at.ch.0.wrapping_mul(8).wrapping_add(x),
            at.ch.1.wrapping_mul(8).wrapping_add(y),
            0,
        );
        let mut subs: HashMap<FeatureType, Rc<Biome>> = HashMap::new();
        let lookup_biome = self.get_map_biome(x, y, lookup.biome.as_deref())?;
        let lookup_ore = self.get_map_biome(x, y, lookup.ore.as_deref())?;
        self.do_pending(
            c,
            &at,
            lookup_biome.as_ref(),
            lookup_ore.as_ref(),
            &mut subs,
        )?;
        let floor_name = {
            let sq_owner = square(c.chunks, &at).expect("square");
            let sq = sq_owner.borrow();
            let f = sq.objects.iter().find(|o| o.borrow().id == floor_id);
            // The floor object itself is never deleted here.
            f.and_then(|o| o.borrow().sprite)
                .and_then(|s| c.env.sprites.sprites[s].name.clone())
                .ok_or_else(|| npe("floor sprite name"))?
        };
        let current: Vec<(FeatureType, Option<ObjectId>)> = {
            let k = Kinds { m: c.env.sprites };
            let sq_owner = square(c.chunks, &at).expect("square");
            let sq = sq_owner.borrow();
            FeatureType::ALL
                .iter()
                .map(|&t| (t, k.current(&sq, t)))
                .collect()
        };
        for &(ty, cur) in &current {
            let Some(cur) = cur else { continue };
            let cur_name = {
                let sq_owner = square(c.chunks, &at).expect("square");
                let sq = sq_owner.borrow();
                // A deleted object has had its sprite cleared.
                match sq
                    .objects
                    .iter()
                    .find(|o| o.borrow().id == cur)
                    .and_then(|o| o.borrow().sprite)
                {
                    Some(s) => c.env.sprites.sprites[s].name.clone().unwrap_or_default(),
                    None => continue,
                }
            };
            let biome = match subs.get(&ty) {
                Some(b) => Some(b.clone()),
                None => self.get_map_biome(x, y, lookup.biome.as_deref())?,
            };
            let Some(mut biome) = biome else {
                return Ok(());
            };
            if self.is_protected(&biome.protected, &cur_name) {
                continue;
            }
            let placement = biome
                .placements
                .as_ref()
                .ok_or_else(|| npe("placements"))?
                .get(&ty);
            if self.can_place(placement, &floor_name)? {
                let mut r = self.apply_biome(c, &at, &biome, ty, &mut rnd)?;
                if r.0 == TileReplacement::Pending {
                    self.do_pending(
                        c,
                        &at,
                        lookup_biome.as_ref(),
                        lookup_ore.as_ref(),
                        &mut subs,
                    )?;
                    biome = match subs.get(&ty) {
                        Some(b) => b.clone(),
                        None => self
                            .get_map_biome(x, y, lookup.biome.as_deref())?
                            .ok_or_else(|| npe("map biome"))?,
                    };
                    r = self.apply_biome(c, &at, &biome, ty, &mut rnd)?;
                }
                let k = Kinds { m: c.env.sprites };
                let sq_owner = square_mut(c.chunks, &at).expect("square");
                let mut sq = sq_owner.borrow_mut();
                if r.0 == TileReplacement::Success {
                    let mut i = 1;
                    while i < sq.objects.len() {
                        let o = sq.objects[i].borrow();
                        if o.sprite != r.1 && (k.tree(&o) || k.bush(&o) || k.grass_like(&o)) {
                            drop(o);
                            sq.remove_object_slot(i);
                        }
                        i += 1;
                    }
                }
                if r.0 == TileReplacement::Delete {
                    let mut i = 1;
                    while i < sq.objects.len() {
                        let o = sq.objects[i].borrow();
                        if k.tree(&o) || k.bush(&o) || k.ores(&o) || k.grass_like(&o) {
                            drop(o);
                            sq.remove_object_slot(i);
                        }
                        i += 1;
                    }
                }
                continue;
            }
            delete(
                &mut square_mut(c.chunks, &at).expect("square").borrow_mut(),
                Some(cur),
            );
        }
        let grasses: Vec<ObjectId> = {
            let k = Kinds { m: c.env.sprites };
            let sq_owner = square(c.chunks, &at).expect("square");
            let sq = sq_owner.borrow();
            sq.objects
                .iter()
                .filter(|o| k.grass_like(&o.borrow()))
                .map(|o| o.borrow().id)
                .collect()
        };
        let n = square(c.chunks, &at)
            .expect("square")
            .borrow()
            .objects
            .len();
        if n - grasses.len() == 1 {
            let biome = match subs.get(&FeatureType::Ore) {
                Some(b) => Some(b.clone()),
                None => self.get_map_biome(x, y, lookup.ore.as_deref())?,
            };
            let Some(mut biome) = biome else {
                return Ok(());
            };
            let placement = biome
                .placements
                .as_ref()
                .ok_or_else(|| npe("placements"))?
                .get(&FeatureType::Ore);
            if !self.can_place(placement, &floor_name)? {
                return Ok(());
            }
            if rocks {
                let mut r = self.apply_biome(c, &at, &biome, FeatureType::Ore, &mut rnd)?;
                if r.0 == TileReplacement::Pending {
                    self.do_pending(
                        c,
                        &at,
                        lookup_biome.as_ref(),
                        lookup_ore.as_ref(),
                        &mut subs,
                    )?;
                    biome = match subs.get(&FeatureType::Ore) {
                        Some(b) => b.clone(),
                        None => self
                            .get_map_biome(x, y, lookup.biome.as_deref())?
                            .ok_or_else(|| npe("map biome"))?,
                    };
                    r = self.apply_biome(c, &at, &biome, FeatureType::Ore, &mut rnd)?;
                }
                if r.0 == TileReplacement::Success {
                    let sq_owner = square_mut(c.chunks, &at).expect("square");
                    let mut sq = sq_owner.borrow_mut();
                    for g in grasses {
                        delete(&mut sq, Some(g));
                    }
                }
            }
        }
        Ok(())
    }

    /// `doPending`: applies what earlier squares left for this one in
    /// `toBeDone` (tiles of multi-square features and `$` markers).
    fn do_pending(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        lookup_biome: Option<&Rc<Biome>>,
        lookup_ore: Option<&Rc<Biome>>,
        subs: &mut HashMap<FeatureType, Rc<Biome>>,
    ) -> JResult<()> {
        let idx = at.tbd();
        for ft in FeatureType::ALL {
            let tile = c.to_be_done[ft.index()][idx].clone();
            let Some(tile) = tile.filter(|t| t != ANY) else {
                c.to_be_done[ft.index()][idx] = None;
                continue;
            };
            let current = {
                let k = Kinds { m: c.env.sprites };
                let sq_owner = square(c.chunks, at).ok_or_else(|| npe("square"))?;
                let sq = sq_owner.borrow();
                [
                    k.current(&sq, FeatureType::Tree),
                    k.current(&sq, FeatureType::Bush),
                    k.current(&sq, FeatureType::Plant),
                ]
            };
            match tile.as_str() {
                SUBBIOME => {
                    let mut rnd2 = self.params.random(
                        at.ch.0.wrapping_mul(8).wrapping_add(at.x),
                        at.ch.1.wrapping_mul(8).wrapping_add(at.y),
                        0,
                    );
                    let tmp = if ft != FeatureType::Ore {
                        lookup_biome.map(|b| &b.sub_biomes)
                    } else {
                        lookup_ore.map(|b| &b.sub_biomes)
                    };
                    let Some(tmp) = tmp.filter(|t| !t.is_empty()) else {
                        continue;
                    };
                    for sub in FeatureType::ALL {
                        let Some(inner) = tmp.get(&sub) else { continue };
                        let list = inner.get(&ft).ok_or_else(|| npe("subbiome feature type"))?;
                        let i = rnd2.next_int_range(0, tmp.len() as i32) as usize;
                        let b = list.get(i).ok_or_else(|| {
                            JavaException("IndexOutOfBoundsException: subbiome".into())
                        })?;
                        subs.insert(sub, b.clone());
                        break;
                    }
                }
                NO_TREE => delete(
                    &mut square_mut(c.chunks, at).expect("square").borrow_mut(),
                    current[0],
                ),
                NO_BUSH => delete(
                    &mut square_mut(c.chunks, at).expect("square").borrow_mut(),
                    current[1],
                ),
                NO_GRASS => delete(
                    &mut square_mut(c.chunks, at).expect("square").borrow_mut(),
                    current[2],
                ),
                _ => {
                    let sq_owner = square_mut(c.chunks, at).expect("square");
                    let mut sq = sq_owner.borrow_mut();
                    for id in current {
                        delete(&mut sq, id);
                    }
                    drop(sq);
                    // `applyTile` with the square's own generator would be
                    // `rnd`; the original passes it but `applyTile` only
                    // uses `Rand` (through `Fix2x`).
                    self.apply_tile_no_rnd(c, at, &tile)?;
                }
            }
        }
        Ok(())
    }

    /// `replaceSquare`: the biome's per-tile replacements (clay floors).
    fn replace_square(&mut self, c: &mut Ctx<'_, '_, '_>, at: At, biomes: &[i32]) -> JResult<()> {
        let (tx, ty) = at.local();
        let Some(lookup) = self
            .biome_map
            .entry(biomes[(ty * 8 + tx) as usize])
            .cloned()
        else {
            return Ok(());
        };
        let Some(biome) = self.get_map_biome(at.x, at.y, lookup.biome.as_deref())? else {
            return Ok(());
        };
        let Some(repl) = biome.replacements.as_ref().filter(|r| !r.is_empty()) else {
            return Ok(());
        };
        if square(c.chunks, &at).is_none() {
            return Ok(());
        }
        let mut rnd = self.params.random(
            at.ch.0.wrapping_mul(8).wrapping_add(at.x),
            at.ch.1.wrapping_mul(8).wrapping_add(at.y),
            0,
        );
        let mut i = 0;
        loop {
            let (id, name) = {
                let sq_owner = square(c.chunks, &at).expect("square");
                let sq = sq_owner.borrow();
                let Some(o) = sq.objects.get(i) else { break };
                let o = o.borrow();
                let s = o.sprite.ok_or_else(|| npe("object sprite"))?;
                (
                    o.id,
                    c.env.sprites.sprites[s].name.clone().unwrap_or_default(),
                )
            };
            i += 1;
            let Some(features) = repl.get(&name) else {
                continue;
            };
            let pre = features.iter().fold(0.0f32, |a, f| a + f.probability);
            let refs: Vec<&Feature> = features.iter().collect();
            let Some(f) = find_feature(&refs, pre, pre, &mut rnd) else {
                continue;
            };
            if f.probability as f64 == 0.0 {
                continue;
            }
            delete(
                &mut square_mut(c.chunks, &at).expect("square").borrow_mut(),
                Some(id),
            );
            let tg = &f.tile_groups[rnd.next_int_bound(f.tile_groups.len() as i32) as usize];
            let tile = tg.tiles[0].clone();
            self.apply_tile(c, &at, &tile, &mut rnd)?;
            let ch = chunk_mut(c.chunks, &at);
            reset_attachments(ch);
            ch.saved
                .attachments_partial
                .get_or_insert_with(Vec::new)
                .push((at.x, at.y, 0));
        }
        Ok(())
    }

    /// `applyBiome`: `setTiles`, then on success `FixStackableObjects` and
    /// the zombie roll.
    fn apply_biome(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        biome: &Rc<Biome>,
        ty: FeatureType,
        rnd: &mut JavaRandom,
    ) -> JResult<(TileReplacement, Option<SpriteIdx>)> {
        let r = self.set_tiles(c, at, biome, ty, rnd)?;
        if r.0 == TileReplacement::Success {
            let m = &*c.env.sprites;
            square_mut(c.chunks, at)
                .expect("square")
                .borrow_mut()
                .fix_stackable_objects(m);
            self.generate_zombies(biome.zombies, at, rnd);
        }
        Ok(r)
    }

    /// `generateZombies(chance, square, rnd)`.
    fn generate_zombies(&mut self, chance: f32, at: &At, rnd: &mut JavaRandom) {
        if rnd.next_float() < chance {
            self.procedural_zombie_squares
                .entry(at.ch)
                .or_default()
                .push((at.x, at.y, 0));
        }
    }

    /// `WorldGenTile.setTiles`: tries feature sizes 8, 4, 2, 1.
    fn set_tiles(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        biome: &Rc<Biome>,
        ty: FeatureType,
        rnd: &mut JavaRandom,
    ) -> JResult<(TileReplacement, Option<SpriteIdx>)> {
        for i in 0..16 {
            let target = 8i32 >> i;
            if target == 0 {
                return Ok((TileReplacement::Failure, None));
            }
            let (lookup, tile) = self.get_biome_tile(c, at, biome, ty, rnd, target, 0)?;
            if lookup == TileLookup::AlreadyPlaced {
                continue;
            }
            if tile.is_none()
                && matches!(
                    lookup,
                    TileLookup::FeaturesListEmpty | TileLookup::NoFeature
                )
            {
                return Ok((TileReplacement::Delete, None));
            }
            let Some(tile) = tile else { continue };
            if [ANY, SUBBIOME, NO_BUSH, NO_GRASS, NO_TREE].contains(&tile.as_str()) {
                return Ok((TileReplacement::Pending, None));
            }
            return self.apply_tile(c, at, &tile, rnd);
        }
        Ok((TileReplacement::Failure, None))
    }

    /// `WorldGenTile.getBiomeTile`.
    #[allow(clippy::too_many_arguments)]
    fn get_biome_tile(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        biome: &Rc<Biome>,
        ty: FeatureType,
        rnd: &mut JavaRandom,
        target: i32,
        depth: i32,
    ) -> JResult<(TileLookup, Option<String>)> {
        let t = ty.index();
        let (tile_x, tile_y) = (at.cc_x, at.cc_y);
        let idx = (tile_x + tile_y * GENERATION_SQUARES) as usize;
        let mut tile = c.to_be_done[t][idx].clone();
        if tile
            .as_deref()
            .is_none_or(|s| s.is_empty() || s == ANY || s == SUBBIOME)
        {
            let Some(features) = biome.features.as_ref() else {
                return Ok((TileLookup::NoFeatures, None));
            };
            let Some(list) = features.get(&ty) else {
                return Ok((TileLookup::NoSpecificType, None));
            };
            if list.is_empty() {
                return Ok((TileLookup::FeaturesListEmpty, None));
            }
            let filtered: Vec<&Feature> = list.iter().filter(|f| f.min_size <= target).collect();
            let pre = list.iter().fold(0.0f32, |a, f| a + f.probability);
            let post = filtered.iter().fold(0.0f32, |a, f| a + f.probability);
            let Some(feature) = find_feature(&filtered, pre, post, rnd) else {
                return Ok((TileLookup::NoFeature, None));
            };
            let groups: Vec<&TileGroup> = feature
                .tile_groups
                .iter()
                .filter(|g| g.sx <= target && g.sy <= target)
                .collect();
            if groups.is_empty() {
                return Ok((TileLookup::TileGroupsEmpty, None));
            }
            let tg = groups[rnd.next_int_bound(groups.len() as i32) as usize];
            if tile_x + tg.sx > GENERATION_SQUARES || tile_y + tg.sy > GENERATION_SQUARES {
                return Ok((TileLookup::TooBig, None));
            }
            let (cx, cy) = at.local();
            if let Some(p) = biome.placements.as_ref() {
                if !self.check_future_squares(c, tg, cx, cy, 0, p.get(&ty))? {
                    return Ok((TileLookup::CantBePlaced, None));
                }
            }
            for ix in 0..tg.sx {
                for iy in 0..tg.sy {
                    let i = (ix + tile_x + (iy + tile_y) * GENERATION_SQUARES) as usize;
                    if c.to_be_done[t][i]
                        .as_deref()
                        .is_some_and(|s| !s.starts_with('$'))
                    {
                        return Ok((TileLookup::AlreadyPlaced, None));
                    }
                }
            }
            for ix in 0..tg.sx {
                for iy in 0..tg.sy {
                    let i = (ix + tile_x + (iy + tile_y) * GENERATION_SQUARES) as usize;
                    c.to_be_done[t][i] = Some(tg.tiles[(ix + iy * tg.sx) as usize].clone());
                }
            }
            tile = c.to_be_done[t][idx].clone();
            if tile.as_deref() == Some(ANY) {
                c.to_be_done[t][idx] = None;
                if depth < 1 {
                    return self.get_biome_tile(c, at, biome, ty, rnd, target, depth + 1);
                }
            }
        }
        Ok((TileLookup::Success, tile))
    }

    /// `checkFutureSquares`. As in the original, the in-chunk coordinates
    /// are looked up relative to the block's first chunk.
    fn check_future_squares(
        &self,
        c: &Ctx<'_, '_, '_>,
        tg: &TileGroup,
        cx: i32,
        cy: i32,
        z: i32,
        placement: Option<&Vec<String>>,
    ) -> JResult<bool> {
        if tg.sx == 1 && tg.sy == 1 {
            return Ok(true);
        }
        let m = &*c.env.sprites;
        let k = Kinds { m };
        for ix in 0..tg.sx {
            for iy in 0..tg.sy {
                if ix == 0 && iy == 0 {
                    continue;
                }
                let Some(sq_owner) = c.chunks.get_grid_square(cx + ix, cy + iy, z) else {
                    return Ok(false);
                };
                let sq = sq_owner.borrow();
                let Some(fi) = sq.floor_index(m) else {
                    return Ok(false);
                };
                let n = sq.objects.len();
                let human = n as i64
                    - k.count(&sq, Kinds::grass_like) as i64
                    - k.count(&sq, Kinds::bush) as i64
                    - k.count(&sq, Kinds::ores) as i64
                    > 1
                    && k.first(&sq, Kinds::tree).is_none();
                if human {
                    return Ok(false);
                }
                let floor_name = k
                    .name(&sq.objects[fi].borrow())
                    .ok_or_else(|| npe("floor name"))?
                    .to_string();
                if !self.can_place(placement, &floor_name)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// `WorldGenTile.applyTile`: creates the tile's object on the square
    /// (`CellLoader.DoTileObjectCreation`), or `carpentry_02_58` with
    /// `FAILURE` when the tile has no definition.
    fn apply_tile(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        tile: &str,
        _rnd: &mut JavaRandom,
    ) -> JResult<(TileReplacement, Option<SpriteIdx>)> {
        self.apply_tile_no_rnd(c, at, tile)
    }

    fn apply_tile_no_rnd(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        tile: &str,
    ) -> JResult<(TileReplacement, Option<SpriteIdx>)> {
        let fixed = c.env.fix2x.fix2x(tile, c.env.rand);
        let (spr, r) = match c.env.sprites.named_map.get(&fixed) {
            Some(&s) => (s, TileReplacement::Success),
            None => {
                let s = *c
                    .env
                    .sprites
                    .named_map
                    .get(MISSING_TILE)
                    .ok_or_else(|| npe(MISSING_TILE))?;
                (s, TileReplacement::Failure)
            }
        };
        let ty = c.env.sprites.sprites[spr].tile_type;
        let meta = c.meta;
        let (lx, ly) = at.local();
        let sq = c
            .chunks
            .get_mut(at.ch.0, at.ch.1)
            .and_then(|ch| ch.get_grid_square_mut(lx, ly, 0))
            .ok_or_else(|| npe("square"))?;
        let mut oe = ObjectEnv {
            sprites: &mut *c.env.sprites,
            rand: &mut *c.env.rand,
            tables: c.env.tables,
            settings: c.env.settings,
            hooks: &mut *c.env.hooks,
        };
        do_tile_object_creation(&mut oe, meta, &sq, spr, ty, tile).map_err(JavaException)?;
        Ok((r, Some(spr)))
    }

    /// `applyOreVeins`: veins of the 21 × 21 cells around the cell origin.
    fn apply_ore_veins(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: &At,
        rnd: &mut JavaRandom,
    ) -> JResult<()> {
        for px in -10..=10 {
            for py in -10..=10 {
                let veins =
                    self.veins
                        .get(&self.params, self.cell_world.0 + px, self.cell_world.1 + py);
                for v in veins.iter() {
                    if !v.is_valid(at.x, at.y, rnd) {
                        continue;
                    }
                    let tiles = self.veins.tiles(v);
                    let tg = &tiles[rnd.next_int_bound(tiles.len() as i32) as usize];
                    let tile = tg.tiles[0].clone();
                    self.apply_tile(c, at, &tile, rnd)?;
                }
            }
        }
        Ok(())
    }

    /// `applyPrefab`.
    #[allow(clippy::too_many_arguments)]
    fn apply_prefab(
        &mut self,
        c: &mut Ctx<'_, '_, '_>,
        at: At,
        prefab: &PrefabStructure,
        biome: Rc<Biome>,
        xmin: i32,
        ymin: i32,
        rnd: &mut JavaRandom,
    ) -> JResult<()> {
        let div = |a: i32, d: i32| -> JResult<i32> {
            if d == 0 {
                Err(JavaException("ArithmeticException: / by zero".into()))
            } else {
                Ok(a.wrapping_abs() % d)
            }
        };
        let px = div(at.x.wrapping_sub(xmin), prefab.dimensions[0])?;
        let py = div(at.y.wrapping_sub(ymin), prefab.dimensions[1])?;
        for cat in PrefabStructure::CATEGORIES {
            if !prefab.schematic.contains_key(cat) {
                continue;
            }
            let r = prefab
                .tile_ref(cat, px, py)
                .ok_or_else(|| JavaException("ArrayIndexOutOfBoundsException: prefab".into()))?;
            if r == 0 {
                if cat == "Floor" {
                    self.set_tiles(c, &at, &biome, FeatureType::Ground, rnd)?;
                }
                continue;
            }
            let tile =
                prefab.tiles.get((r - 1) as usize).cloned().ok_or_else(|| {
                    JavaException("IndexOutOfBoundsException: prefab tile".into())
                })?;
            self.apply_tile(c, &at, &tile, rnd)?;
        }
        let m = &*c.env.sprites;
        square_mut(c.chunks, &at)
            .expect("square")
            .borrow_mut()
            .fix_stackable_objects(m);
        self.generate_zombies(prefab.zombies, &at, rnd);
        Ok(())
    }

    /// The constant of selection axis `axis` the noise value falls in, tested
    /// in the original's order; the axis' `NONE` otherwise.
    fn select(&self, axis: usize, noise: f64) -> JResult<u8> {
        let order: &[&str] = match axis {
            0 => &["FOREST", "LIGHT_FOREST", "PLAIN"],
            1 => &["FLOWER", "GRASS"],
            2 => &["DRY", "REGULAR", "FAT"],
            3 => &["HOT", "MEDIUM", "COLD"],
            4 => &["FLOODING", "RAIN", "DRY"],
            _ => &["VERY_HIGH", "HIGH", "MEDIUM", "LOW", "VERY_LOW"],
        };
        let a = AXES[axis];
        for name in order {
            let i = a.index_of(name).expect("constant");
            let range = self.selection[axis]
                .get(&i)
                .ok_or_else(|| npe("selection range"))?;
            let (lo, hi) = (
                *range.first().ok_or_else(|| npe("range"))?,
                *range.get(1).ok_or_else(|| npe("range"))?,
            );
            if noise >= lo && noise < hi {
                return Ok(i);
            }
        }
        Ok(a.none())
    }

    fn pick(&self, list: &[Rc<Biome>], idx: &[usize], selector: f64) -> JResult<Rc<Biome>> {
        if idx.is_empty() {
            return Ok(self.default_biome.clone());
        }
        let i = (selector * idx.len() as f64) as i32 as usize;
        idx.get(i)
            .map(|&i| list[i].clone())
            .ok_or_else(|| JavaException("IndexOutOfBoundsException: biome".into()))
    }

    /// `getBiome(x, y)` (`BiomeRegistry.get` over all six axes).
    pub fn get_biome(&self, x: i32, y: i32) -> JResult<Rc<Biome>> {
        let n = self.simplex.noise(x as f64, y as f64);
        let sel = self.simplex.selector(x as f64, y as f64);
        let key = [
            self.select(0, n.landscape)?,
            self.select(1, n.plant)?,
            self.select(2, n.bush)?,
            self.select(3, n.temperature)?,
            self.select(4, n.hygrometry)?,
            self.select(5, n.ore)?,
        ];
        let idx = self
            .biome_cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                (0..self.biomes.len())
                    .filter(|&i| (0..6).all(|a| self.biomes[i].has(a, key[a])))
                    .collect()
            })
            .clone();
        self.pick(&self.biomes, &idx, sel)
    }

    /// `getMapBiome(x, y, filter)`: map biomes whose name starts with
    /// `filter`, narrowed by the bush and ore noise.
    pub fn get_map_biome(
        &self,
        x: i32,
        y: i32,
        filter: Option<&str>,
    ) -> JResult<Option<Rc<Biome>>> {
        let Some(filter) = filter else {
            return Ok(None);
        };
        let n = self.simplex.noise(x as f64, y as f64);
        let sel = self.simplex.selector(x as f64, y as f64);
        let bush = self.select(2, n.bush)?;
        let ore = self.select(5, n.ore)?;
        let idx = self
            .map_biome_cache
            .borrow_mut()
            .entry((filter.to_string(), bush, ore))
            .or_insert_with(|| {
                (0..self.biomes_map.len())
                    .filter(|&i| {
                        let b = &self.biomes_map[i];
                        b.name.starts_with(filter) && b.has(2, bush) && b.has(5, ore)
                    })
                    .collect()
            })
            .clone();
        self.pick(&self.biomes_map, &idx, sel).map(Some)
    }

    /// `priority(tile, remote)`.
    pub fn priority(&self, tile: &str, remote: &str) -> bool {
        match (self.priorities.get(tile), self.priorities.get(remote)) {
            (Some(a), Some(b)) => a < b,
            _ => false,
        }
    }

    /// `isProtected(list, tile)`.
    pub fn is_protected(&self, list: &[String], tile: &str) -> bool {
        list.iter().any(|p| self.wild.matches(p, tile))
    }

    /// `WorldGenUtils.canPlace(placement, floorName)`: the last matching
    /// pattern decides (`!pattern` excludes).
    pub fn can_place(&self, placement: Option<&Vec<String>>, floor: &str) -> JResult<bool> {
        let list = placement.ok_or_else(|| npe("placement"))?;
        let mut ok = false;
        for s in list {
            let (check, pat) = match s.strip_prefix('!') {
                Some(p) => (false, p),
                None => (true, s.as_str()),
            };
            if self.wild.matches(pat, floor) {
                ok = check;
            }
        }
        Ok(ok)
    }

    /// `cleanChunk(chunk, material, filter)`: on floors whose
    /// `FloorMaterial` matches, removes objects whose sprite name starts
    /// with `filter`.
    fn clean_chunk(&self, ch: &mut IsoChunk, m: &IsoSpriteManager, material: &str, filter: &str) {
        let re = wildcard_regex(material);
        for x in 0..8 {
            for y in 0..8 {
                let Some(sq_owner) = ch.get_grid_square_mut(x, y, 0) else {
                    continue;
                };
                let mut sq = sq_owner.borrow_mut();
                let Some(fi) = sq.floor_index(m) else {
                    continue;
                };
                let fs = sq.objects[fi].borrow().sprite.expect("floor sprite");
                let mat = m.sprites[fs].properties.get(&m.alias, "FloorMaterial");
                if mat.is_some_and(|mat| self.wild.matches_regex(&re, mat)) {
                    sq.retain_objects(|o| {
                        !o.borrow()
                            .sprite
                            .and_then(|s| m.sprites[s].name.as_deref())
                            .is_some_and(|n| n.starts_with(filter))
                    });
                }
            }
        }
    }
}

/// The chunk state `genRandomChunk` sets before generating squares: level
/// 0 only, blending done, attachments pending and the `WORLDGEN`
/// generation status.
pub fn start_random_chunk(ch: &mut IsoChunk) {
    ch.set_min_max_level(0, 0);
    ch.saved.blending_done_full = true;
    ch.saved.blending_done_partial = false;
    ch.saved.attachments_done_full = false;
    ch.saved.attachments_state = [false; 5];
    ch.add_modded(ChunkGenerationStatus::Worldgen);
}

/// `IsoChunk.hasEmptySquaresOnLevelZero()`: a level-0 square that is
/// missing or empty with nothing below it.
pub fn has_empty_squares_on_level_zero(ch: &IsoChunk) -> bool {
    for y in 0..8 {
        for x in 0..8 {
            let filled = ch
                .get_grid_square(x, y, 0)
                .is_some_and(|s| !s.borrow().objects.is_empty());
            let below = (ch.min_level..0).rev().any(|z| {
                ch.get_grid_square(x, y, z)
                    .is_some_and(|s| !s.borrow().objects.is_empty())
            });
            if filled || below {
                continue;
            }
            return true;
        }
    }
    false
}

/// Names of the axis constants (for diagnostics).
pub fn axis_names() -> [&'static [&'static str]; 6] {
    [
        LANDSCAPE.names,
        PLANT.names,
        BUSH.names,
        TEMPERATURE.names,
        HYGROMETRY.names,
        ORE_LEVEL.names,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_chunks_are_marked_worldgen() {
        let mut ch = IsoChunk::new(3, 4);
        ch.set_square(0, 0, -1, Some((IsoGridSquare::new(24, 32, -1)).into()));
        assert!(has_empty_squares_on_level_zero(&ch));
        start_random_chunk(&mut ch);
        assert!(ch
            .generation_status
            .contains(ChunkGenerationStatus::Worldgen));
        assert_eq!((ch.min_level, ch.max_level), (0, 0));
        assert!(ch.saved.blending_done_full);
        assert!(!ch.saved.attachments_done_full);
        assert_eq!(ch.saved.attachments_state, [false; 5]);
    }

    #[test]
    fn map_chunk_without_holes_is_not_random() {
        let mut ch = IsoChunk::new(0, 0);
        for x in 0..8 {
            for y in 0..8 {
                let mut sq = IsoGridSquare::new(x, y, 0);
                sq.objects
                    .push(pz_world::object_slot::ObjectSlot::new(IsoObject::new(
                        0,
                        ObjectKind::Object,
                        None,
                    )));
                ch.set_square(x, y, 0, Some((sq).into()));
            }
        }
        assert!(!has_empty_squares_on_level_zero(&ch));
    }
}
