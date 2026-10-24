//! `zombie.basements`: procedural basements.
//!
//! At world creation (`Basements.beforeLoadMetaGrid`) every basement spawn
//! location of a map (the `Basement` zones of `objects.lua` and the
//! `procedural_basement_spawn_locations` of the map's `basements.lua`) gets
//! one of the map's basement definitions that fits (stair direction,
//! bounds, no overlap with basements placed before), chosen with a
//! generator seeded by the world seed, plus the access piece the location
//! names (`ba_*`, which reworks the house above the stairs). When a
//! brand-new chunk is loaded (`Basements.onNewChunkLoaded`) the parts of the
//! placed `.pzby` pieces that fall in the chunk are stamped into it
//! (`NewMapBinaryFile.SpawnBasementInChunk`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::{TableRef, Value};
use pz_world::cell_loader::{do_tile_object_creation, ObjectEnv};
use pz_world::chunk::{ChunkLoadEnv, IsoChunk, IsoGridSquare};
use pz_world::{BuildingDef, BuildingIdx, GameRand, IsoMetaGrid, RoomDef, RoomIdx, RoomRect};

use crate::java::{hashmap_order, JavaRandom};
use crate::pzby::{PzbyChunk, PzbyHeader, CHUNK_DIM};

/// `BasementDefinition` (a basement or an access piece).
#[derive(Clone, Debug)]
pub struct BasementDefinition {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub stairx: i32,
    pub stairy: i32,
    pub north: bool,
    pub header: Option<Rc<PzbyHeader>>,
}

/// `BasementSpawnLocation`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BasementSpawnLocation {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    pub stair_x: i32,
    pub stair_y: i32,
    pub north: bool,
    /// `specificBasement` (`choices`).
    pub specific: Option<Vec<String>>,
    pub access: Option<String>,
}

/// `BasementPlacement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasementPlacement {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
    pub h: i32,
    pub name: String,
}

/// `BasementsPerMap`.
#[derive(Clone, Debug, Default)]
pub struct BasementsPerMap {
    pub map_id: String,
    pub definitions: Vec<BasementDefinition>,
    pub definition_by_name: HashMap<String, usize>,
    pub access_definitions: Vec<BasementDefinition>,
    pub access_by_name: HashMap<String, usize>,
    pub spawn_locations: Vec<BasementSpawnLocation>,
}

/// One call of the Lua API (`Basements.getAPIv1()`).
enum ApiCall {
    Access(String, TableRef),
    Basements(String, TableRef),
    Spawns(String, TableRef),
}

/// `Basements.getInstance()`.
pub struct Basements {
    fs: Rc<GameFs>,
    /// Buildings added to the meta grid and buildings merged into a
    /// neighbour by [`Basements::add_buildings_to_meta_grid`].
    pub meta_buildings: (usize, usize),
    /// In `getOrCreatePerMap` order.
    pub per_map: Vec<BasementsPerMap>,
    pub placements: Vec<BasementPlacement>,
    /// Errors met while loading definitions (missing `.pzby` files and the
    /// like).
    pub errors: Vec<String>,
}

fn num(t: &TableRef, k: &str) -> Option<i32> {
    t.get_str(k).as_num().map(|d| d as i32)
}

fn table_entries(t: &TableRef) -> Vec<(Value, Value)> {
    t.entries()
}

impl Basements {
    pub fn new(fs: Rc<GameFs>) -> Self {
        Basements {
            fs,
            per_map: Vec::new(),
            placements: Vec::new(),
            errors: Vec::new(),
            meta_buildings: (0, 0),
        }
    }

    /// `getOrCreatePerMap(mapID)`.
    pub fn per_map_mut(&mut self, map_id: &str) -> &mut BasementsPerMap {
        let id = map_id.trim();
        if let Some(i) = self.per_map.iter().position(|p| p.map_id == id) {
            return &mut self.per_map[i];
        }
        self.per_map.push(BasementsPerMap {
            map_id: id.to_string(),
            ..Default::default()
        });
        self.per_map.last_mut().unwrap()
    }

    /// Runs each lot directory's `basements.lua` (last to first, like
    /// `loadMapBasementLuaFiles`) with the `Basements.getAPIv1()` calls
    /// recorded, then applies them.
    pub fn load_lua(&mut self, lot_dirs: &[&str]) -> Result<(), String> {
        let calls: Rc<RefCell<Vec<ApiCall>>> = Rc::new(RefCell::new(Vec::new()));
        let mut lua = crate::lua::WorldGenLua::new(self.fs.clone()).map_err(|e| e.to_string())?;
        let api = TableRef::new();
        let rec = |name: &'static str, calls: Rc<RefCell<Vec<ApiCall>>>| {
            Value::Native(pz_kahlua::NativeFunction::new(name, move |c| {
                let map = pz_kahlua::stdlib::string_arg(c, 2, name)?.to_string();
                let t = c.arg(2).as_table().cloned().unwrap_or_default();
                calls.borrow_mut().push(match name {
                    "addAccessDefinitions" => ApiCall::Access(map, t),
                    "addBasementDefinitions" => ApiCall::Basements(map, t),
                    _ => ApiCall::Spawns(map, t),
                });
                Ok(0)
            }))
        };
        for n in [
            "addAccessDefinitions",
            "addBasementDefinitions",
            "addSpawnLocations",
        ] {
            api.set_str(n, rec(n, calls.clone()));
        }
        let basements = TableRef::new();
        let api2 = api.clone();
        basements.set_str(
            "getAPIv1",
            Value::Native(pz_kahlua::NativeFunction::new("getAPIv1", move |c| {
                Ok(c.push(Value::Table(api2.clone())))
            })),
        );
        lua.thread.globals().set_str("Basements", basements);
        for dir in lot_dirs.iter().rev() {
            if let Some(p) = self
                .fs
                .resolve(&format!("media/maps/{dir}/basements.lua"))
                .filter(|p| p.exists())
            {
                lua.run(&p);
            }
        }
        self.errors.extend(lua.errors());
        let calls = std::mem::take(&mut *calls.borrow_mut());
        for call in calls {
            match call {
                ApiCall::Access(map, t) => {
                    let defs = parse_definitions(&t)?;
                    let pm = self.per_map_mut(&map);
                    for d in defs {
                        pm.access_by_name
                            .insert(d.name.clone(), pm.access_definitions.len());
                        pm.access_definitions.push(d);
                    }
                }
                ApiCall::Basements(map, t) => {
                    let defs = parse_definitions(&t)?;
                    let pm = self.per_map_mut(&map);
                    for d in defs {
                        pm.definition_by_name
                            .insert(d.name.clone(), pm.definitions.len());
                        pm.definitions.push(d);
                    }
                }
                ApiCall::Spawns(map, t) => {
                    let spawns = parse_spawn_locations(&t)?;
                    self.per_map_mut(&map).spawn_locations.extend(spawns);
                }
            }
        }
        Ok(())
    }

    /// `registerBasementSpawnLocation` for a `Basement` map zone.
    pub fn register_spawn_location(&mut self, map_id: &str, loc: BasementSpawnLocation) {
        self.per_map_mut(map_id).spawn_locations.push(loc);
    }

    /// `loadBasementDefinitionHeaders` and `loadBasementAccessDefinitionHeaders`:
    /// reads each definition's `.pzby` header; a basement whose header is
    /// missing or has other than one building is dropped (from the lookup
    /// by name; the original's per-map list keeps it without a header).
    pub fn load_headers(&mut self, rand: &mut dyn GameRand) {
        let fs = self.fs.clone();
        let mut errors = Vec::new();
        for pm in &mut self.per_map {
            for d in &mut pm.definitions {
                match load_header(&fs, "media/binmap", &d.name, rand) {
                    Ok(h) if h.buildings.len() == 1 => {
                        d.width = h.buildings[0].w();
                        d.height = h.buildings[0].h();
                        d.header = Some(Rc::new(h));
                    }
                    Ok(_) => {
                        pm.definition_by_name.remove(&d.name);
                    }
                    Err(e) => {
                        errors.push(format!("{}: {e}", d.name));
                        pm.definition_by_name.remove(&d.name);
                    }
                }
            }
            for d in &mut pm.access_definitions {
                match load_header(&fs, "media/basement_access", &d.name, rand) {
                    Ok(h) => d.header = Some(Rc::new(h)),
                    Err(e) => {
                        errors.push(format!("{}: {e}", d.name));
                        pm.access_by_name.remove(&d.name);
                    }
                }
            }
        }
        self.errors.extend(errors);
    }

    /// `calculateBasementPlacements()`: per map (in `HashMap` order of the
    /// map ids), with `new RandSeeded(worldSeed)`.
    pub fn calculate_placements(&mut self, meta: &IsoMetaGrid, seed: i64) {
        let mut rnd = JavaRandom::new(seed);
        let mut overlap = Overlap::default();
        let ids: Vec<String> = self.per_map.iter().map(|p| p.map_id.clone()).collect();
        // The access lookup is the global one (`basementAccessDefinitionByName`):
        // every map's access definitions, later maps replacing earlier ones.
        let mut access: HashMap<String, BasementDefinition> = HashMap::new();
        for pm in &self.per_map {
            for (name, &i) in &pm.access_by_name {
                access.insert(name.clone(), pm.access_definitions[i].clone());
            }
        }
        for i in hashmap_order(&ids) {
            let pm = self.per_map[i].clone();
            let mut errors = Vec::new();
            self.calculate_for_map(meta, &pm, &access, &mut overlap, &mut rnd, &mut errors);
            self.errors.extend(errors);
        }
    }

    fn calculate_for_map(
        &mut self,
        meta: &IsoMetaGrid,
        pm: &BasementsPerMap,
        access: &HashMap<String, BasementDefinition>,
        overlap: &mut Overlap,
        rnd: &mut JavaRandom,
        errors: &mut Vec<String>,
    ) {
        for loc in &pm.spawn_locations {
            let mut choices: Vec<&BasementDefinition> = Vec::new();
            match loc.specific.as_ref().filter(|s| !s.is_empty()) {
                None => {
                    for d in &pm.definitions {
                        match can_place_at(meta, &pm.map_id, d, loc, overlap) {
                            Ok(true) => choices.push(d),
                            Ok(false) => {}
                            Err(e) => errors.push(e),
                        }
                    }
                }
                Some(names) => {
                    for n in names {
                        let Some(&i) = pm.definition_by_name.get(n) else {
                            continue;
                        };
                        let d = &pm.definitions[i];
                        if d.header.is_none() {
                            continue;
                        }
                        if can_place_at(meta, &pm.map_id, d, loc, overlap).unwrap_or(false) {
                            choices.push(d);
                        }
                    }
                }
            }
            if choices.is_empty() {
                continue;
            }
            let d = choices[rnd.next_int_bound(choices.len() as i32) as usize];
            let header = d.header.as_ref().expect("checked header");
            let (x, y) = if loc.w > 1 && loc.h > 1 {
                (
                    loc.x + loc.stair_x - d.stairx,
                    loc.y + loc.stair_y - d.stairy,
                )
            } else {
                (loc.x - d.stairx, loc.y - d.stairy)
            };
            let p = BasementPlacement {
                x,
                y,
                z: loc.z - header.levels,
                w: d.width,
                h: d.height,
                name: d.name.clone(),
            };
            overlap.add(header.clone(), p.x, p.y, p.z);
            let (sx, sy) = (p.x + d.stairx, p.y + d.stairy);
            self.placements.push(p);
            let Some(a) = loc.access.as_ref().and_then(|n| access.get(n)) else {
                continue;
            };
            if a.header.is_none() {
                continue;
            }
            self.placements.push(BasementPlacement {
                x: sx - a.stairx,
                y: sy - a.stairy,
                z: loc.z,
                w: a.width,
                h: a.height,
                name: a.name.clone(),
            });
        }
    }

    /// The header of a placement's piece: a basement definition (the
    /// global `basementDefinitionByName`), else an access definition.
    fn placement_header(&self, name: &str) -> Option<Rc<PzbyHeader>> {
        let find = |access: bool| {
            self.per_map.iter().rev().find_map(|pm| {
                let (by_name, defs) = if access {
                    (&pm.access_by_name, &pm.access_definitions)
                } else {
                    (&pm.definition_by_name, &pm.definitions)
                };
                by_name.get(name).and_then(|&i| defs[i].header.clone())
            })
        };
        find(false).or_else(|| find(true))
    }

    /// `createBasementBuildingDefs`, `addBasementBuildingDefsToLotHeaders`
    /// and `addBasementBuildingDefsToMetaGrid`: each placement becomes a
    /// building of the meta grid (rooms moved to the placement and its
    /// bottom level). A building that does not go below level 0 (an access
    /// piece) and touches exactly one other such building is merged into
    /// it. `rand` is `Rand` at world loading (`new BuildingDef()` draws).
    /// Returns the number of new buildings and of merged ones.
    pub fn add_buildings_to_meta_grid(
        &self,
        meta: &mut IsoMetaGrid,
        rand: &mut dyn GameRand,
    ) -> (usize, usize) {
        let mut defs: Vec<BuildingIdx> = Vec::new();
        for p in &self.placements {
            let Some(header) = self.placement_header(&p.name) else {
                continue;
            };
            if header.buildings.is_empty() || meta.get_cell_data(p.x / 256, p.y / 256).is_none() {
                continue;
            }
            let mut def = BuildingDef::new(rand);
            def.id = defs.len() as i64;
            let (mut rooms, mut empty) = (Vec::new(), Vec::new());
            for r in &header.rooms {
                let mut nr = RoomDef::new(rooms.len() as i64, r.name.clone());
                nr.level = p.z + r.level;
                for &(x, y, w, h) in &r.rects {
                    nr.rects.push(RoomRect::new(p.x + x, p.y + y, w, h));
                }
                nr.CalculateBounds();
                if r.is_empty_outside() {
                    empty.push(nr);
                } else {
                    rooms.push(nr);
                }
            }
            defs.push(meta.insert_building(def, rooms, empty));
        }
        // `mergedRooms` (a HashMap keyed by identity in the original; kept
        // here in first-merge order).
        let mut merged: Vec<(BuildingIdx, Vec<RoomIdx>, Vec<RoomIdx>)> = Vec::new();
        let mut kept = Vec::new();
        let mut n_merged = 0;
        for b in defs {
            let d = &meta.building_defs[b.0 as usize];
            if meta.get_cell_data(d.x / 256, d.y / 256).is_none() {
                kept.push(b);
                continue;
            }
            if let Some(m) = meta.get_building_to_merge_with(b) {
                let d = &meta.building_defs[b.0 as usize];
                let (r, e) = (d.rooms.clone(), d.emptyoutside.clone());
                n_merged += 1;
                if meta.merge_buildings(b, m) {
                    match merged.iter_mut().find(|x| x.0 == m) {
                        Some(x) => {
                            x.1.extend(r);
                            x.2.extend(e);
                        }
                        None => merged.push((m, r, e)),
                    }
                }
                continue;
            }
            meta.add_building_to_lot_header(b);
            kept.push(b);
        }
        let counts = (kept.len(), n_merged);
        for b in kept {
            meta.add_building_to_meta_grid(b);
        }
        for (m, r, e) in merged {
            meta.add_merged_rooms_to_meta_grid(m, &r, &e);
        }
        counts
    }

    /// `onNewChunkLoaded(chunk)`: stamps every placement whose bounds (plus
    /// one square) touch the chunk.
    pub fn on_new_chunk_loaded(
        &mut self,
        chunk: &mut IsoChunk,
        meta: &IsoMetaGrid,
        env: &mut ChunkLoadEnv<'_>,
    ) {
        let (cx, cy) = (chunk.wx * 8, chunk.wy * 8);
        for i in 0..self.placements.len() {
            let p = self.placements[i].clone();
            let (w, h) = (p.w + 1, p.h + 1);
            if !(p.x + w > cx && p.x < cx + 8 && p.y + h > cy && p.y < cy + 8) {
                continue;
            }
            if let Err(e) =
                spawn_basement_in_chunk(&self.fs, chunk, meta, env, &p.name, p.x, p.y, p.z)
            {
                self.errors
                    .push(format!("SpawnBasementInChunk {}: {e}", p.name));
            }
        }
    }
}

fn load_header(
    fs: &GameFs,
    dir: &str,
    name: &str,
    rand: &mut dyn GameRand,
) -> std::io::Result<PzbyHeader> {
    let rel = format!("{dir}/{name}.pzby");
    let path: PathBuf = fs
        .resolve(&rel)
        .filter(|p| p.exists())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel))?;
    PzbyHeader::load(&path, rand)
}

/// `BasementsV1.addBasementDefinitions/addAccessDefinitions` table entries.
fn parse_definitions(t: &TableRef) -> Result<Vec<BasementDefinition>, String> {
    let mut out = Vec::new();
    for (k, v) in table_entries(t) {
        let name = k
            .as_str()
            .ok_or("ClassCastException: definition name")?
            .to_string();
        let d = v.as_table().ok_or("ClassCastException: definition")?;
        let get = |key: &str| num(d, key).ok_or(format!("NullPointerException: {name}.{key}"));
        out.push(BasementDefinition {
            width: get("width")?,
            height: get("height")?,
            stairx: get("stairx")?,
            stairy: get("stairy")?,
            north: d.get_str("stairDir").as_str() == Some("N"),
            name,
            header: None,
        });
    }
    Ok(out)
}

/// `BasementsV1.addSpawnLocations` table entries.
fn parse_spawn_locations(t: &TableRef) -> Result<Vec<BasementSpawnLocation>, String> {
    let mut out = Vec::new();
    for (_, v) in table_entries(t) {
        let l = v.as_table().ok_or("ClassCastException: spawn location")?;
        let get = |key: &str| num(l, key).ok_or(format!("NullPointerException: spawn.{key}"));
        let specific = l.get_str("choices").as_table().map(|c| {
            table_entries(c)
                .into_iter()
                .filter_map(|(_, v)| v.as_str().map(|s| pz_world::java::java_trim(s).to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        });
        out.push(BasementSpawnLocation {
            x: get("x")?,
            y: get("y")?,
            z: num(l, "z").unwrap_or(0),
            w: 1,
            h: 1,
            stair_x: 0,
            stair_y: 0,
            north: l.get_str("stairDir").as_str() == Some("N"),
            specific,
            access: l.get_str("access").as_str().map(str::to_string),
        });
    }
    Ok(out)
}

/// A `Basement` zone of `objects.lua` (`registerBasementSpawnLocation`).
pub fn spawn_location_from_zone(t: &TableRef) -> BasementSpawnLocation {
    let n = |k: &str| t.get_str(k).as_num().unwrap_or(0.0) as i32;
    let mut loc = BasementSpawnLocation {
        x: n("x"),
        y: n("y"),
        z: n("z"),
        w: n("width"),
        h: n("height"),
        north: true,
        ..Default::default()
    };
    if let Some(p) = t.get_str("properties").as_table() {
        if let Some(v) = p.get_str("StairX").as_num() {
            loc.stair_x = v as i32;
        }
        if let Some(v) = p.get_str("StairY").as_num() {
            loc.stair_y = v as i32;
        }
        if let Some(s) = p.get_str("StairDirection").as_str() {
            loc.north = s == "N";
        }
        if let Some(s) = p.get_str("Access").as_str() {
            loc.access = Some(s.to_string());
        }
    }
    loc
}

/// `canPlaceAt(mapID, definition, location, overlap)`.
fn can_place_at(
    meta: &IsoMetaGrid,
    map_id: &str,
    d: &BasementDefinition,
    loc: &BasementSpawnLocation,
    overlap: &Overlap,
) -> Result<bool, String> {
    if d.north != loc.north {
        return Ok(false);
    }
    let header = d
        .header
        .as_ref()
        .ok_or(format!("NullPointerException: no header for {}", d.name))?;
    let bx = loc.x + loc.stair_x - d.stairx;
    let by = loc.y + loc.stair_y - d.stairy;
    let bz = loc.z - header.levels;
    if loc.w > 1 && loc.h > 1 {
        let inside = bx >= loc.x
            && bx + d.width <= loc.x + loc.w
            && by >= loc.y
            && by + d.height <= loc.y + loc.h;
        if !inside {
            return Ok(false);
        }
    }
    // `isCellFromThisMap`.
    let hn = pz_world::map_files::header_name(loc.x / 256, loc.y / 256);
    let from_map = meta.info_headers.get(&hn).is_some_and(|&mf| {
        meta.map_files[mf]
            .map_directory_name
            .eq_ignore_ascii_case(map_id)
    });
    if !from_map {
        return Ok(false);
    }
    Ok(!overlap.check(header, bx, by, bz))
}

/// `BasementOverlap`. The original buckets basements by 20-square chunks of
/// 200-square cells; a pair can only overlap where their bounds do, so
/// checking every placed basement gives the same answers.
#[derive(Default)]
struct Overlap {
    placed: Vec<(Rc<PzbyHeader>, i32, i32, i32)>,
}

impl Overlap {
    fn add(&mut self, h: Rc<PzbyHeader>, x: i32, y: i32, z: i32) {
        self.placed.push((h, x, y, z));
    }

    /// `checkOverlap`: as in the original, two basements whose level
    /// ranges and bounds intersect always count as overlapping (the
    /// per-room test's result is ignored).
    fn check(&self, h: &PzbyHeader, x: i32, y: i32, z: i32) -> bool {
        let b = &h.buildings[0];
        self.placed.iter().any(|(ph, px, py, pz)| {
            let pb = &ph.buildings[0];
            let (z_min1, z_max1) = (*pz, pz + pb.max_level(&ph.rooms));
            let (z_min2, z_max2) = (z, z + b.max_level(&h.rooms));
            if z_min2 > z_max1 || z_max2 < z_min1 {
                return false;
            }
            if x >= px + pb.w() || x + b.w() <= *px {
                return false;
            }
            !(y >= py + pb.h() || y + b.h() <= *py)
        })
    }
}

/// `NewMapBinaryFile.SpawnBasementInChunk(chunk, name, x, y, bottomZ)`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_basement_in_chunk(
    fs: &GameFs,
    chunk: &mut IsoChunk,
    meta: &IsoMetaGrid,
    env: &mut ChunkLoadEnv<'_>,
    name: &str,
    x: i32,
    y: i32,
    bottom_z: i32,
) -> std::io::Result<()> {
    let access = name.starts_with("ba_");
    let dir = if access {
        "media/basement_access"
    } else {
        "media/binmap"
    };
    let header = load_header(fs, dir, name, env.rand)?;
    let (cx, cy) = (chunk.wx * 8, chunk.wy * 8);
    let bx1 = cx.max(x) - x;
    let by1 = cy.max(y) - y;
    let bx2 = (cx + 8).min(x + header.width * CHUNK_DIM) - 1 - x;
    let by2 = (cy + 8).min(y + header.height * CHUNK_DIM) - 1 - y;
    let div = |a: i32| a / CHUNK_DIM;
    for ly in div(by1)..=div(by2) {
        for lx in div(bx1)..=div(bx2) {
            let data = header.load_chunk(lx, ly)?;
            add_to_iso_chunk(
                chunk,
                meta,
                env,
                &header,
                &data,
                x + lx * CHUNK_DIM,
                y + ly * CHUNK_DIM,
                bottom_z,
            )
            .map_err(std::io::Error::other)?;
        }
    }
    Ok(())
}

/// `NewMapBinaryFile.addToIsoChunk`.
#[allow(clippy::too_many_arguments)]
fn add_to_iso_chunk(
    chunk: &mut IsoChunk,
    meta: &IsoMetaGrid,
    env: &mut ChunkLoadEnv<'_>,
    header: &PzbyHeader,
    data: &PzbyChunk,
    tx: i32,
    ty: i32,
    tz: i32,
) -> Result<(), String> {
    let (cx, cy) = (chunk.wx * 8, chunk.wy * 8);
    let x1 = cx.max(tx);
    let y1 = cy.max(ty);
    let x2 = (cx + 8).min(tx + CHUNK_DIM);
    let y2 = (cy + 8).min(ty + CHUNK_DIM);
    let t = &env.sprites.tables;
    let flag = |n: &str| t.flag_from_string(n);
    let (wall_w, wall_n, wall_nw, wall_se) =
        (flag("WallW"), flag("WallN"), flag("WallNW"), flag("WallSE"));
    for x in x1..x2 {
        for y in y1..y2 {
            for z in tz..tz + header.levels {
                let Some(ints) = data.tiles(x - tx, y - ty, z - tz).filter(|v| !v.is_empty())
                else {
                    continue;
                };
                if chunk.get_grid_square(x - cx, y - cy, z).is_none() {
                    let mut sq = IsoGridSquare::new(x, y, z);
                    sq.associated_building = meta.get_associated_building_at(x, y)?;
                    chunk.set_square(x - cx, y - cy, z, Some(sq));
                }
                let room_id = meta.get_room_at(x, y, z).map_or(-1, |r| r.id);
                let square = chunk
                    .get_grid_square_mut(x - cx, y - cy, z)
                    .expect("square");
                square.room_id = room_id;
                if room_id != -1 {
                    meta.get_room_by_id(room_id, env.rand);
                }
                let attributes = data.attribute(x - tx, y - ty, z - tz);
                let keep_floors = attributes & 1 != 0;
                let keep_walls = attributes & 2 != 0;
                let keep_other = attributes & 4 != 0;
                if keep_floors || keep_walls || keep_other {
                    let m = &*env.sprites;
                    let mut i = square.objects.len();
                    while i > 0 {
                        i -= 1;
                        let o = &square.objects[i];
                        let spr = o.sprite.map(|s| &m.sprites[s]);
                        let is_floor = spr.is_some_and(|s| s.solidfloor);
                        let has = |f: Option<pz_world::sprite::IsoFlagType>| {
                            f.is_some_and(|f| spr.is_some_and(|s| s.properties.has(f)))
                        };
                        let is_wall = spr.is_some()
                            && (has(wall_w) || has(wall_n) || has(wall_nw) || has(wall_se));
                        let is_other = !is_floor && !is_wall;
                        let keep = (keep_floors && is_floor)
                            || (keep_walls && is_wall)
                            || (keep_other && is_other);
                        if keep {
                            continue;
                        }
                        let id = o.id;
                        square.objects.remove(i);
                        square.special_objects.retain(|&s| s != id);
                    }
                } else {
                    square.objects.clear();
                    square.special_objects.clear();
                }
                for &n in ints {
                    let tile = header
                        .used_tile_names
                        .get(n as usize)
                        .ok_or("IndexOutOfBoundsException: tile index")?
                        .clone();
                    let Some(&spr) = env.sprites.named_map.get(&tile) else {
                        eprintln!("Missing tile definition: {tile}");
                        continue;
                    };
                    let ty = env.sprites.sprites[spr].tile_type;
                    let mut oe = ObjectEnv {
                        sprites: &mut *env.sprites,
                        rand: &mut *env.rand,
                        tables: env.tables,
                        settings: env.settings,
                        hooks: &mut *env.hooks,
                    };
                    let square = chunk
                        .get_grid_square_mut(x - cx, y - cy, z)
                        .expect("square");
                    do_tile_object_creation(&mut oe, meta, square, spr, ty, &tile)?;
                }
                let m = &*env.sprites;
                chunk
                    .get_grid_square_mut(x - cx, y - cy, z)
                    .expect("square")
                    .fix_stackable_objects(m);
            }
            remove_floors_above_stairs(chunk, env, x - cx, y - cy, tz + header.levels);
        }
    }
    Ok(())
}

/// `removeFloorsAboveStairs(square)`: the first floor of a square over
/// stairs (or a slope) is removed.
fn remove_floors_above_stairs(
    chunk: &mut IsoChunk,
    env: &ChunkLoadEnv<'_>,
    lx: i32,
    ly: i32,
    z: i32,
) {
    if chunk.get_grid_square(lx, ly, z).is_none() {
        return;
    }
    let m = &*env.sprites;
    let Some(below) = chunk.get_grid_square_mut(lx, ly, z - 1) else {
        return;
    };
    below.recalc_properties(m);
    if !below.has_stairs(m) && !below.has_sloped_surface() {
        return;
    }
    let square = chunk.get_grid_square_mut(lx, ly, z).expect("square");
    let f = m.tables.flag.solidfloor;
    if let Some(i) = square
        .objects
        .iter()
        .position(|o| o.sprite.is_some_and(|s| m.sprites[s].properties.has(f)))
    {
        square.objects.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(entries: Vec<(&str, Value)>) -> TableRef {
        let t = TableRef::new();
        for (k, v) in entries {
            t.rawset(Value::str(k), v);
        }
        t
    }

    #[test]
    fn definitions_and_spawns_from_lua_tables() {
        let defs = t(vec![(
            "b1",
            Value::Table(t(vec![
                ("width", Value::Num(6.0)),
                ("height", Value::Num(10.0)),
                ("stairx", Value::Num(0.0)),
                ("stairy", Value::Num(1.0)),
                ("stairDir", Value::str("N")),
            ])),
        )]);
        let d = parse_definitions(&defs).unwrap();
        assert_eq!((d[0].width, d[0].stairy, d[0].north), (6, 1, true));
        let zone = t(vec![
            ("x", Value::Num(100.0)),
            ("y", Value::Num(200.0)),
            ("z", Value::Num(0.0)),
            ("width", Value::Num(8.0)),
            ("height", Value::Num(12.0)),
            (
                "properties",
                Value::Table(t(vec![
                    ("StairX", Value::Num(7.0)),
                    ("StairDirection", Value::str("W")),
                    ("Access", Value::str("ba_x")),
                ])),
            ),
        ]);
        let l = spawn_location_from_zone(&zone);
        assert_eq!(
            (l.x, l.y, l.w, l.h, l.stair_x, l.north),
            (100, 200, 8, 12, 7, false)
        );
        assert_eq!(l.access.as_deref(), Some("ba_x"));
    }

    #[test]
    fn placed_basement_becomes_a_meta_grid_building() {
        let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
        meta.create_cell(0, 0);
        let room = crate::pzby::PzbyRoom {
            name: "basement".into(),
            level: 0,
            rects: vec![(0, 0, 3, 3)],
            x: 0,
            y: 0,
            x2: 0,
            y2: 0,
        };
        let header = Rc::new(PzbyHeader::synthetic(1, vec![room]));
        let mut b = Basements::new(Rc::new(GameFs::new("/nonexistent")));
        let pm = b.per_map_mut("m");
        pm.definition_by_name.insert("b".into(), 0);
        pm.definitions.push(BasementDefinition {
            name: "b".into(),
            width: 3,
            height: 3,
            stairx: 0,
            stairy: 0,
            north: true,
            header: Some(header),
        });
        b.placements.push(BasementPlacement {
            x: 10,
            y: 20,
            z: -1,
            w: 3,
            h: 3,
            name: "b".into(),
        });
        let counts = b.add_buildings_to_meta_grid(&mut meta, &mut JavaRandom::new(1));
        assert_eq!(counts, (1, 0));
        let r = meta.get_room_at(11, 21, -1).expect("basement room");
        assert_eq!(r.level, -1);
        assert!(r.building.is_some());
        assert!(meta.get_room_at(11, 21, 0).is_none());
        assert!(meta.get_room_at(13, 21, -1).is_none());
    }
}
