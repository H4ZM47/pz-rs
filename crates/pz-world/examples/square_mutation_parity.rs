//! The square mutations ([`pz_world::square_mutation`]) and player-built
//! buildings ([`pz_world::isoregion::metagrid`]) against the original's
//! `IsoGridSquare` / `IsoRegions` / `WorldRegionToMetaGrid`, run by a local
//! harness on the same chunk files with the same pseudo-random sequence of
//! calls (no game data is part of this repository).
//!
//! ```text
//! square_mutation_parity <game dir> <sprite tables> <save tables> <save dir> <oracle steps.txt> <out dir>
//! ```
//!
//! The chunks in a `WINDOW`-wide square around the save's median chunk are
//! loaded (with `IsoWindow.load`'s sprite changes) and every square's
//! properties recalculated; the meta grid has empty cells around the
//! window. Each step picks a position and an operation (`AddTileObject`,
//! `AddSpecialObject`, `RemoveTileObject`, `transmitRemoveItemFromSquare`,
//! `addFloor`, `RemoveTileObjectErosionNoRecalc`, `createNewGridSquare`, or
//! a small room of walls, floors and a floor above), runs one region
//! worker loop with the main thread's buffer-swap side, and writes the hook
//! calls, the statics they set, the `Rand` seed and the squares around the
//! position; every `FULL_EVERY` steps the regions and the meta grid's
//! buildings. Writes `rust_steps.txt` and prints the first difference.

use pz_world::ObjectSlot;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_world::isoregion::metagrid::BuildingCtx;
use pz_world::isoregion::{IsoRegions, WorldRegionRef};
use pz_world::objects::ObjectId;
use pz_world::save::chunk_file::{apply_loaded_window_sprites, chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::square_mutation::{
    adjacent_squares, DoorTables, LuaEvent, MutationEnv, ObjectRef, PathfindSquareHooks,
    RenderSquareHooks, SquareMutationHooks, SquarePos, WorldCall,
};
use pz_world::{
    BuildingIdx, GameRand, IsoCell, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager,
    ObjectKind, SpriteTables,
};

/// `java.util.Random`.
struct JavaRandom(u64);

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn bits(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i64 as i32
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut r = self.bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.bits(31);
        }
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

/// The hook calls of one step, in the categories the harness prints.
#[derive(Default)]
struct Rec {
    names: HashMap<usize, String>,
    lua: Vec<String>,
    pathfind: Vec<SquarePos>,
    chunks: Vec<(i32, i32)>,
    collision: Vec<(i32, i32)>,
    los: [bool; 4],
    recalc_light: f32,
    light_update: f32,
    occlusion: i32,
    dirty: i32,
    powered: Option<bool>,
    fluid: Option<f32>,
    /// Sprites with `lightR`, `lightG` and `lightB`.
    light_sprites: std::collections::HashSet<usize>,
    /// Objects whose `addLightSourceToWorld` made a light source.
    lit: std::collections::HashSet<ObjectRef>,
}

impl Rec {
    fn reset(&mut self) {
        self.los = [false; 4];
        self.recalc_light = 12345.0;
        self.light_update = 0.0;
        self.occlusion = 0;
        self.dirty = 0;
    }
    fn name(&self, o: &IsoObject) -> String {
        match o.sprite.tile_projection() {
            None if matches!(o.sprite, pz_world::objects::ObjectSprite::ArchiveUnmanaged(_)) => "nil".into(),
            None => "nosprite".into(),
            Some(s) => self.names.get(&s).cloned().unwrap_or_else(|| "nil".into()),
        }
    }
}

/// One of the three hook views onto the shared recorder.
struct H(Rc<RefCell<Rec>>);

impl PathfindSquareHooks for H {
    fn square_changed(&mut self, cell: &IsoCell, sq: SquarePos) {
        let mut r = self.0.borrow_mut();
        for a in adjacent_squares(cell, sq).into_iter().flatten() {
            r.pathfind.push((a.borrow().x, a.borrow().y, a.borrow().z));
        }
        r.pathfind.push(sq);
    }
    fn map_collision_square_changed(&mut self, sq: pz_world::SquareRef) {
        let sq = sq.borrow();
        self.0.borrow_mut().collision.push((sq.x, sq.y));
    }
    fn add_chunk_to_world(&mut self, _: &IsoCell, wx: i32, wy: i32) {
        self.0.borrow_mut().chunks.push((wx, wy));
    }
}

impl RenderSquareHooks for H {
    fn fbo_render_chunk(&self) -> bool {
        true
    }
    fn los_cache_cleared(&mut self, p: i32) {
        self.0.borrow_mut().los[p as usize] = true;
    }
    fn set_recalc_light_time(&mut self, t: f32) {
        self.0.borrow_mut().recalc_light = t;
    }
    fn dirty_global_lights(&mut self) {
        self.0.borrow_mut().dirty += 1;
    }
    fn light_source_update(&mut self) {
        self.0.borrow_mut().light_update = 100.0;
    }
    fn grid_occlusion_square_changed(&mut self) {
        self.0.borrow_mut().occlusion += 1;
    }
    fn buildings_changed(&mut self) {
        let mut r = self.0.borrow_mut();
        r.light_update = 100.0;
        r.los = [true; 4];
        r.dirty += 1;
    }
}

impl SquareMutationHooks for H {
    // This harness compares a data/trace profile; no live registry is supplied.
    fn try_world_call(
        &mut self,
        obj: ObjectRef,
        object: pz_world::IsoObjectRef,
        rand: &mut dyn pz_world::GameRand,
        call: WorldCall,
    ) -> pz_entity::runtime_load::HostResult<()> {
        self.world_call(obj, object, rand, call);
        Ok(())
    }

    fn lua_event(&mut self, sq: pz_world::SquareRef, ev: LuaEvent) {
        let sq = sq.borrow();
        let mut r = self.0.borrow_mut();
        let r = &mut *r;
        let s = match ev {
            LuaEvent::ObjectAboutToBeRemoved { index, .. } => {
                format!(
                    "OnObjectAboutToBeRemoved {}",
                    r.name(&sq.objects[index].borrow())
                )
            }
            LuaEvent::TileRemoved { object } => format!(
                "OnTileRemoved {}",
                object.map_or("nil".to_string(), |o| r.name(&o.borrow()))
            ),
            LuaEvent::ContainerUpdate => "OnContainerUpdate".into(),
        };
        r.lua.push(s);
    }
    fn container_powered(&mut self, _: ObjectRef, _: &IsoObject, _: usize) -> bool {
        self.0.borrow().powered.unwrap_or(false)
    }
    fn fluid_amount(&mut self, _: ObjectRef, _: &IsoObject) -> f32 {
        self.0.borrow().fluid.unwrap_or(0.0)
    }
    // What `IsoCell.addLamppost` / `removeLamppost` do with the light source
    // `addLightSourceToWorld` makes for a sprite with light colours.
    fn world_call(
        &mut self,
        r: ObjectRef,
        o: pz_world::IsoObjectRef,
        _: &mut dyn GameRand,
        c: WorldCall,
    ) {
        let o = o.borrow();
        let mut rec = self.0.borrow_mut();
        if let WorldCall::SwitchLamppost { add: false, .. } = c {
            // `IsoCell.removeLamppost(light)`.
            rec.recalc_light = -1.0;
            rec.dirty += 1;
            rec.light_update = 100.0;
            return;
        }
        let WorldCall::LightSource { add } = c else {
            return;
        };
        let lights = |o: &IsoObject| {
            !matches!(
                o.kind,
                ObjectKind::Fire(_)
                    | ObjectKind::Fireplace(_)
                    | ObjectKind::LightSwitch(_)
                    | ObjectKind::Pushable(_)
                    | ObjectKind::Television(_)
            )
        };
        let changed = if add {
            lights(&o)
                && o.sprite.tile_projection().is_some_and(|s| rec.light_sprites.contains(&s))
                && rec.lit.insert(r)
        } else {
            rec.lit.remove(&r)
        };
        if changed {
            rec.recalc_light = -1.0;
            rec.dirty += 1;
            rec.light_update = 100.0;
        }
    }
}

/// `Float.toString` for the values the statics take.
fn jf(f: f32) -> String {
    if f.fract() == 0.0 && f.abs() < 1e7 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n as u64) as usize
    }
}

struct World {
    cell: IsoCell,
    meta: IsoMetaGrid,
    sprites: IsoSpriteManager,
    regions: IsoRegions,
    rand: JavaRandom,
    doors: DoorTables,
    rec: Rc<RefCell<Rec>>,
    hw: H,
    hp: H,
    hr: H,
    swaps: i32,
    order: Vec<(i32, i32)>,
    bd_n: HashMap<BuildingIdx, usize>,
    out: String,
}

impl World {
    fn env(&mut self) -> MutationEnv<'_> {
        MutationEnv {
            cell: &mut self.cell,
            meta: &self.meta,
            sprites: &mut self.sprites,
            regions: &mut self.regions,
            rand: &mut self.rand,
            doors: &self.doors,
            world: &mut self.hw,
            pathfind: &mut self.hp,
            render: &mut self.hr,
            num_players: 1,
        }
    }

    fn tick(&mut self) {
        let mut ctx = BuildingCtx {
            cell: &mut self.cell,
            meta: &mut self.meta,
            sprites: &self.sprites,
            rand: &mut self.rand,
            world: &mut self.hw,
            pathfind: &mut self.hp,
            render: &mut self.hr,
        };
        if self.regions.update_with_buildings(&mut ctx, None) {
            self.swaps += 1;
        }
    }

    fn sq(&self, p: SquarePos) -> Option<pz_world::SquareRef> {
        self.cell.get_grid_square(p.0, p.1, p.2)
    }

    fn name_of(&self, o: &IsoObject) -> String {
        match o.sprite.tile_projection() {
            None => "null".into(),
            Some(s) => self.sprites.sprites[s]
                .name
                .clone()
                .unwrap_or_else(|| "null".into()),
        }
    }

    fn new_object(&self, sprite: &str) -> IsoObject {
        let s = self.sprites.named_map.get(sprite).copied();
        let mut o = IsoObject::new(0, ObjectKind::Object, s);
        o.tile = Some(sprite.to_string());
        o
    }

    fn index_of(&self, p: SquarePos, id: Option<ObjectId>) -> i32 {
        id.and_then(|id| self.sq(p).and_then(|sq| sq.borrow().object_index(id)))
            .map_or(-1, |i| i as i32)
    }

    fn queries(&self, p: SquarePos, id: Option<ObjectId>) -> String {
        let mut s = String::new();
        let Some(o) = id.and_then(|id| {
            self.sq(p)?
                .borrow()
                .objects
                .iter()
                .find(|o| o.borrow().id == id)
                .map(ObjectSlot::ref_handle)
        }) else {
            return s;
        };
        let o = o.borrow();
        if o.containers()
            .any(|c| matches!(c.kind.as_deref(), Some("fridge" | "freezer")))
        {
            write!(s, " powered={}", self.rec.borrow().powered.unwrap_or(false)).unwrap();
        }
        let piped = self.sprites.tables.flag_from_string("waterPiped");
        if o.sprite.tile_projection()
            .zip(piped)
            .is_some_and(|(sp, f)| self.sprites.sprites[sp].properties.has(f))
        {
            write!(
                s,
                " fluid={}",
                self.rec.borrow().fluid.unwrap_or(0.0).to_bits() as i32
            )
            .unwrap();
        }
        s
    }

    fn create(&mut self, p: SquarePos) -> bool {
        self.env().create_new_grid_square(p, true).is_some()
    }
}

struct Pools {
    pool: Vec<String>,
    floors: Vec<String>,
    wall_n: String,
    wall_w: String,
    floor: String,
}

fn do_op(w: &mut World, pools: &Pools, rng: &mut Lcg, p: SquarePos, op: usize) -> String {
    if op >= 95 {
        return build_room(w, pools, rng, p);
    }
    if w.sq(p).is_none() {
        return format!("create {}", w.create(p));
    }
    let n = w.sq(p).unwrap().borrow().objects.len();
    if op < 42 {
        let special = op >= 35;
        let s = pools.pool[rng.next(pools.pool.len())].clone();
        let index = if rng.next(3) == 0 {
            rng.next(n + 2) as i32 - 1
        } else {
            -1
        };
        let o = w.new_object(&s);
        let id = if special {
            w.env().add_special_object(p, (o).into(), index)
        } else {
            w.env().add_tile_object(p, (o).into(), index)
        };
        let id = match id { Ok(id)=>id, Err(error)=>return format!("world add host fault: {error:?}") };
        return format!(
            "{} {s} {index} -> {}{}",
            if special { "addSpecial" } else { "add" },
            w.index_of(p, id),
            w.queries(p, id)
        );
    }
    if op < 80 {
        let what = if op < 72 { "remove" } else { "transmitRemove" };
        if n == 0 {
            return format!("{what} none");
        }
        let i = rng.next(n);
        let o_owner = w.sq(p).unwrap().borrow().objects[i].ref_handle();
        let o = o_owner.borrow();
        let (id, name) = (o.id, w.name_of(&o));
        let name = if op < 72 {
            format!("{}:{name}", o.kind.class_name())
        } else {
            name
        };
        drop(o);
        let (r, _) = if op < 72 {
            w.env()
                .remove_tile_object(p, id)
                .expect("archive parity fixture removal")
        } else {
            w.env()
                .transmit_remove_item_from_square(p, id, true)
                .expect("archive parity fixture removal")
        };
        return format!("{what} {i} {name} -> {r}");
    }
    if op < 92 {
        let s = pools.floors[rng.next(pools.floors.len())].clone();
        let id = w
            .env()
            .add_floor(p, &s)
            .expect("archive parity fixture add floor");
        return format!("addFloor {s} -> {}", w.index_of(p, id));
    }
    if n == 0 {
        return "erosionRemove none".into();
    }
    let i = rng.next(n);
    let o_owner = w.sq(p).unwrap().borrow().objects[i].ref_handle();
    let o = o_owner.borrow();
    let (id, name) = (o.id, w.name_of(&o));
    drop(o);
    let (r, _) = w
        .env()
        .remove_tile_object_erosion_no_recalc(p, id)
        .expect("archive parity fixture erosion removal");
    format!("erosionRemove {i} {name} -> {r}")
}

fn build_room(w: &mut World, pools: &Pools, rng: &mut Lcg, p: SquarePos) -> String {
    let (ww, h) = (1 + rng.next(3) as i32, 1 + rng.next(3) as i32);
    let z = if p.2 == 2 { 1 } else { p.2 };
    let (x0, y0) = (p.0, p.1);
    let mut n = 0;
    let mut wall = |w: &mut World, x: i32, y: i32, spr: &str| {
        if w.sq((x, y, z)).is_none() && !w.create((x, y, z)) {
            return;
        }
        let o = w.new_object(spr);
        w.env().add_tile_object((x, y, z), (o).into(), -1)
            .expect("explicit tile data profile lifecycle");
        n += 1;
    };
    for y in y0..y0 + h {
        let mut x = x0;
        while x <= x0 + ww {
            wall(w, x, y, &pools.wall_w);
            x += ww;
        }
    }
    for x in x0..x0 + ww {
        let mut y = y0;
        while y <= y0 + h {
            wall(w, x, y, &pools.wall_n);
            y += h;
        }
    }
    for y in y0..y0 + h {
        for x in x0..x0 + ww {
            for zz in z..=z + 1 {
                if w.sq((x, y, zz)).is_none() && !w.create((x, y, zz)) {
                    continue;
                }
                w.env()
                    .add_floor((x, y, zz), &pools.floor)
                    .expect("archive parity fixture add floor");
                n += 1;
            }
        }
    }
    format!("room {ww}x{h} z{z} ops={n}")
}

fn drain(w: &mut World) {
    let rc = w.rec.clone();
    let mut r = rc.borrow_mut();
    for l in r.lua.drain(..) {
        writeln!(w.out, " lua {l}").unwrap();
    }
    if !r.pathfind.is_empty() {
        let v: Vec<String> = r
            .pathfind
            .drain(..)
            .map(|(x, y, z)| format!("{x},{y},{z}"))
            .collect();
        writeln!(w.out, " pathfind {}", v.join(" ")).unwrap();
    }
    if !r.chunks.is_empty() {
        let v: Vec<String> = r
            .chunks
            .drain(..)
            .map(|(x, y)| format!("{x},{y}"))
            .collect();
        writeln!(w.out, " pathfindChunks {}", v.join(" ")).unwrap();
    }
    if !r.collision.is_empty() {
        let v: Vec<String> = r
            .collision
            .drain(..)
            .map(|(x, y)| format!("{x},{y}"))
            .collect();
        writeln!(w.out, " collision {}", v.join(" ")).unwrap();
    }
    writeln!(
        w.out,
        " statics los={}{}{}{} recalcLight={} lightUpdate={} occlusion+{} dirtyLights+{} rand={:x}",
        r.los[0],
        r.los[1],
        r.los[2],
        r.los[3],
        jf(r.recalc_light),
        jf(r.light_update),
        r.occlusion,
        r.dirty,
        w.rand.0
    )
    .unwrap();
}

fn flags(w: &World, sq: &IsoGridSquare) -> String {
    let t = &w.sprites.tables;
    let mut l: Vec<String> = sq
        .properties
        .flags_list(t)
        .into_iter()
        .map(|f| t.flag_name(f).to_string())
        .collect();
    l.sort();
    let mut pv: Vec<String> = sq
        .properties
        .property_pairs(&w.sprites.alias)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    pv.sort();
    format!("{} props={}", l.join(","), pv.join(","))
}

fn dump_squares(w: &mut World, p: SquarePos) {
    let mut out = String::new();
    for z in -1..=3 {
        for y in p.1 - 1..=p.1 + 1 {
            for x in p.0 - 1..=p.0 + 1 {
                let Some(sq) = w.sq((x, y, z)) else {
                    continue;
                };
                let assoc = match sq.borrow().associated_building {
                    None => "-".to_string(),
                    Some(b) => w.bd_n.get(&b).map_or(-2, |&n| n as i64).to_string(),
                };
                write!(
                    out,
                    " sq {x},{y},{z} room={} roof={} burnt={} col={:x} path={:x} vis={:x} assoc={assoc} flags={} objs=[",
                    sq.borrow().room_id,
                    sq.borrow().have_roof,
                    sq.borrow().burnt_out,
                    sq.borrow().collide_matrix,
                    sq.borrow().path_matrix,
                    sq.borrow().vision_matrix,
                    flags(w, &(sq).borrow())
                )
                .unwrap();
                let objs: Vec<String> = sq
                    .borrow()
                    .objects
                    .iter()
                    .map(ObjectSlot::borrow)
                    .map(|o| {
                        format!(
                            "{}:{}:{:x}:{}",
                            o.kind.class_name(),
                            w.name_of(&o),
                            o.render_y_offset.to_bits(),
                            o.container_count()
                        )
                    })
                    .collect();
                let special: Vec<String> = sq
                    .borrow()
                    .special_objects
                    .iter()
                    .map(|&id| {
                        sq.borrow()
                            .object_index(id)
                            .map_or(-1, |i| i as i32)
                            .to_string()
                    })
                    .collect();
                writeln!(
                    out,
                    "{}] special=[{}] rflags={}",
                    objs.join(" "),
                    special.join(" "),
                    if z < 0 {
                        "-".to_string()
                    } else {
                        w.regions.get_square_flags(x, y, z).to_string()
                    }
                )
                .unwrap();
            }
        }
    }
    w.out.push_str(&out);
}

fn dump_regions(w: &mut World) {
    let root = w.regions.data_root();
    let m = &root.region_manager;
    let mut cr_n: Vec<pz_world::isoregion::ChunkRegionRef> = Vec::new();
    let mut wr_n: Vec<WorldRegionRef> = Vec::new();
    let mut ncr = |r: Option<pz_world::isoregion::ChunkRegionRef>| -> i64 {
        let Some(r) = r else { return -1 };
        if let Some(i) = cr_n.iter().position(|&x| x == r) {
            return i as i64;
        }
        cr_n.push(r);
        cr_n.len() as i64 - 1
    };
    fn nwr(wr_n: &mut Vec<WorldRegionRef>, r: Option<WorldRegionRef>) -> i64 {
        let Some(r) = r else { return -1 };
        if let Some(i) = wr_n.iter().position(|&x| x == r) {
            return i as i64;
        }
        wr_n.push(r);
        wr_n.len() as i64 - 1
    }
    let mut out = String::new();
    writeln!(
        out,
        "regions cr={} wr={} nextId={} swaps={}",
        m.chunk_region_count(),
        m.world_region_count(),
        m.next_id(),
        w.swaps
    )
    .unwrap();
    for &(cx, cy) in &w.order {
        let Some(dci) = root.get_data_chunk(cx, cy) else {
            continue;
        };
        let dc = root.chunk(dci);
        for z in 0..32 {
            if !dc.is_active(z) {
                continue;
            }
            let mut fl = String::new();
            let mut rg = String::new();
            for y in 0..8 {
                for x in 0..8 {
                    write!(fl, "{:02x}", dc.get_square_checked(x, y, z, true) as u8).unwrap();
                    write!(rg, " {}", ncr(dc.get_iso_chunk_region(x, y, z))).unwrap();
                }
            }
            writeln!(out, " chunk {cx},{cy} z{z} flags={fl} regions{rg}").unwrap();
            for &cr in dc.chunk_regions(z) {
                let c = m.chunk_region(cr);
                writeln!(
                    out,
                    "  cr {} sz={} roof={} encl={} wr={}",
                    ncr(Some(cr)),
                    c.square_size(),
                    c.roof_cnt(),
                    m.chunk_region_is_enclosed(cr),
                    nwr(&mut wr_n, c.iso_world_region())
                )
                .unwrap();
            }
        }
    }
    let mut i = 0;
    while i < wr_n.len() {
        let r = wr_n[i];
        let wr = m.world_region(r);
        let nb: Vec<String> = wr
            .neighbors()
            .to_vec()
            .into_iter()
            .map(|n| nwr(&mut wr_n, Some(n)).to_string())
            .collect();
        let bdef = match wr.building_def() {
            None => "-".to_string(),
            Some(b) => w.bd_n.get(&b).map_or(-2, |&n| n as i64).to_string(),
        };
        writeln!(
            out,
            " wr {i} sz={} roof={} encl={} fog={} nb=[{}] bdef={bdef}",
            wr.square_size(),
            wr.roof_cnt(),
            m.world_region_is_enclosed(r),
            m.world_region_is_fog_mask(r),
            nb.join(",")
        )
        .unwrap();
        i += 1;
    }
    w.out.push_str(&out);
}

fn dump_meta(w: &mut World) {
    let m = &w.meta;
    w.bd_n.clear();
    let mut out = String::new();
    writeln!(out, "meta buildings={}", m.buildings.len()).unwrap();
    for x in 0..m.width {
        for y in 0..m.height {
            let Some(mc) = m.get_cell(x, y) else {
                continue;
            };
            if mc.buildings.is_empty() && mc.room_list.is_empty() {
                continue;
            }
            writeln!(
                out,
                " cell {},{} rooms={} roomByMetaId={} buildingByMetaId={}",
                mc.get_x(),
                mc.get_y(),
                mc.rooms.len(),
                mc.room_by_meta_id.len(),
                mc.building_by_meta_id.len()
            )
            .unwrap();
            for &b in &mc.buildings {
                let n = w.bd_n.len();
                let n = *w.bd_n.entry(b).or_insert(n);
                let d = &m.building_defs[b.0 as usize];
                write!(
                    out,
                    "  b {n} id={} meta={} user={} key={} bounds={},{},{},{} rooms=",
                    d.id, d.meta_id, d.user_defined, d.key_id, d.x, d.y, d.x2, d.y2
                )
                .unwrap();
                for r in &d.rooms {
                    let r = &m.room_defs[r.0 as usize];
                    write!(out, "{{{} {} l{}", r.id, r.meta_id, r.level).unwrap();
                    for rr in &r.rects {
                        write!(out, " {},{},{},{}", rr.x, rr.y, rr.w, rr.h).unwrap();
                    }
                    out.push('}');
                }
                writeln!(out, " eo={}", d.emptyoutside.len()).unwrap();
            }
            write!(out, "  roomList").unwrap();
            for r in &mc.room_list {
                write!(out, " {}", m.room_defs[r.0 as usize].id).unwrap();
            }
            out.push('\n');
        }
    }
    w.out.push_str(&out);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = std::fs::read_to_string(&a[5]).unwrap_or_default();
    let out_dir = Path::new(&a[6]);
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let settings = SaveSettings::default();
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .expect("built-in registries");
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .expect("scripts");
    let dict = WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
        .unwrap();
    let names = pz_inventory::factory::parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let mut registry = ScriptItems::new(&sm, dict, &names);
    registry.blood_body_parts = save_tables.blood_body_parts;
    registry.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let doors = DoorTables::from_jar(&jar).unwrap();
    let ectx = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
        ..Default::default()
    };
    let ectx: &'static pz_entity::BasicContext = Box::leak(Box::new(ectx));
    registry.components = Box::new(pz_inventory::component::EntityCodec { ctx: ectx });
    let mut all = Vec::new();
    for d in std::fs::read_dir(save_dir.join("map")).unwrap() {
        let d = d.unwrap();
        let Ok(wx) = d.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for f in std::fs::read_dir(d.path()).unwrap() {
            let n = f.unwrap().file_name().to_string_lossy().to_string();
            if let Some(wy) = n.strip_suffix(".bin").and_then(|s| s.parse::<i32>().ok()) {
                all.push((wx, wy));
            }
        }
    }
    all.sort();
    let win: i32 = std::env::var("WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(13);
    let cx = all[all.len() / 2].0;
    let mut ys: Vec<i32> = all.iter().map(|p| p.1).collect();
    ys.sort();
    let cy = ys[ys.len() / 2];
    let (min_wx, min_wy) = (cx - win / 2, cy - win / 2);
    let mut cell = IsoCell::default();
    let mut order = Vec::new();
    {
        let ctx = SaveCtx {
            sprites: &sprites,
            tables: &save_tables,
            settings: &settings,
            items: &registry,
            hooks: &NoSaveHooks,
            meta: None,
        };
        for &(wx, wy) in &all {
            if wx < min_wx || wx >= min_wx + win || wy < min_wy || wy >= min_wy + win {
                continue;
            }
            let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
            match load_chunk(&bytes, wx, wy, &ctx) {
                Ok(l) => {
                    cell.chunks.insert((wx, wy), l.chunk);
                    order.push((wx, wy));
                }
                Err(e) => eprintln!("load {wx},{wy}: {e}"),
            }
        }
    }
    drop(registry);
    for &(wx, wy) in &order {
        apply_loaded_window_sprites(&cell.chunks[&(wx, wy)], &mut sprites);
    }
    // Start from the harness's loaded objects where the chunk-file loader
    // differs (barricade sprites from `IsoBarricade.chooseSprite`, world
    // items the original drops on load): `init.txt` next to the oracle steps.
    let init = Path::new(&a[5]).with_file_name("init.txt");
    let mut aligned = (0, 0);
    for l in std::fs::read_to_string(init).unwrap_or_default().lines() {
        let mut it = l.split(' ');
        let p: Vec<i32> = it
            .next()
            .unwrap()
            .split(',')
            .map(|v| v.parse().unwrap())
            .collect();
        let java: Vec<(&str, &str)> = it.map(|o| o.split_once(':').unwrap()).collect();
        let Some(sq) = cell.get_grid_square_mut(p[0], p[1], p[2]) else {
            continue;
        };
        let mut i = 0;
        while i < sq.borrow().objects.len() {
            let o_owner = sq.borrow().objects[i].ref_handle();
            let mut o = o_owner.borrow_mut();
            let name = o.sprite.tile_projection().and_then(|s| sprites.sprites[s].name.clone());
            match java.get(i) {
                Some(&(class, spr)) if class == o.kind.class_name() => {
                    if name.as_deref().unwrap_or("null") != spr {
                        if let Some(&s) = sprites.named_map.get(spr) {
                            o.sprite = Some(s).into();
                            aligned.0 += 1;
                        }
                    }
                    i += 1;
                }
                _ => {
                    drop(o);
                    let removed = sq.borrow_mut().remove_object_slot(i);
                    let id = removed.borrow().id;
                    let entry = sq
                        .borrow()
                        .world_objects
                        .borrow_mut()
                        .remove_first_object(&o_owner);
                    if let Some(pz_world::WorldObjectEntry::Object(owner)) = entry {
                        sq.borrow_mut().retire_object(ObjectSlot::retain(&owner));
                    }
                    sq.borrow_mut().special_objects.retain(|&w| w != id);
                    aligned.1 += 1;
                }
            }
        }
    }
    eprintln!("aligned sprites={} removed={}", aligned.0, aligned.1);
    for &(wx, wy) in &order {
        let ch = cell.chunks.get_mut(&(wx, wy)).unwrap();
        for z in ch.min_level..=ch.max_level {
            for y in 0..8 {
                for x in 0..8 {
                    if let Some(sq) = ch.get_grid_square_mut(x, y, z) {
                        sq.borrow_mut().recalc_properties(&sprites);
                        // `IsoLightSwitch.load` ends with `createLightSource`
                        // for a switch outside a room (the blank meta grid
                        // has none); the chunk-file loader does not yet.
                        let (sx, sy, sz) = (sq.borrow().x, sq.borrow().y, sq.borrow().z);
                        for mut o in sq.borrow().objects.iter().map(ObjectSlot::borrow_mut) {
                            if let ObjectKind::LightSwitch(s) = &mut o.kind {
                                s.lights.push(pz_world::objects::IsoLightSource {
                                    x: sx,
                                    y: sy,
                                    z: sz,
                                    r: 0.9,
                                    g: 0.8,
                                    b: 0.7,
                                    radius: 8,
                                    active: s.activated,
                                    hydro_powered: true,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    let (cmin_x, cmin_y) = (min_wx.div_euclid(32) - 1, min_wy.div_euclid(32) - 1);
    let (cmax_x, cmax_y) = (
        (min_wx + win - 1).div_euclid(32) + 1,
        (min_wy + win - 1).div_euclid(32) + 1,
    );
    let mut meta = IsoMetaGrid::blank(cmin_x, cmin_y, cmax_x, cmax_y);
    for x in cmin_x..=cmax_x {
        for y in cmin_y..=cmax_y {
            meta.create_cell(x, y);
        }
    }
    let region_dir = out_dir.join("isoregiondata");
    let _ = std::fs::remove_dir_all(&region_dir);
    std::fs::create_dir_all(&region_dir).unwrap();
    let regions = IsoRegions::init(Some(out_dir), win);
    let mut rec = Rec::default();
    for (s, sp) in sprites.sprites.iter().enumerate() {
        if let Some(n) = &sp.name {
            rec.names.insert(s, n.clone());
        }
        if ["lightR", "lightG", "lightB"]
            .iter()
            .all(|p| sprites.has_prop(sp, p))
        {
            rec.light_sprites.insert(s);
        }
    }
    let rec = Rc::new(RefCell::new(rec));
    let mut w = World {
        cell,
        meta,
        sprites,
        regions,
        rand: JavaRandom::new(12345),
        doors,
        hw: H(rec.clone()),
        hp: H(rec.clone()),
        hr: H(rec.clone()),
        rec,
        swaps: 0,
        order,
        bd_n: HashMap::new(),
        out: String::new(),
    };
    w.tick();
    // Sprite pools, as the harness builds them.
    let names: BTreeSet<String> = w.sprites.named_map.keys().cloned().collect();
    let prefixes = [
        "walls_",
        "fixtures_doors_frames_",
        "floors_",
        "furniture_",
        "fixtures_",
        "location_",
        "carpentry_",
        "fencing_",
        "vegetation_",
        "street_",
        "appliances_",
        "crafted_",
        "constructedobjects_",
    ];
    let f = w.sprites.tables.flag;
    let mut pools = Pools {
        pool: Vec::new(),
        floors: Vec::new(),
        wall_n: String::new(),
        wall_w: String::new(),
        floor: String::new(),
    };
    for n in &names {
        if prefixes.iter().any(|p| n.starts_with(p)) {
            pools.pool.push(n.clone());
        }
        if n.starts_with("floors_") {
            pools.floors.push(n.clone());
        }
        let p = &w.sprites.sprites[w.sprites.named_map[n]].properties;
        let (cn, cw) = (p.has(f.cutN), p.has(f.cutW));
        if n.starts_with("walls_exterior_wooden_") {
            if pools.wall_n.is_empty() && cn && !cw {
                pools.wall_n = n.clone();
            }
            if pools.wall_w.is_empty() && cw && !cn {
                pools.wall_w = n.clone();
            }
        }
        if pools.floor.is_empty()
            && n.starts_with("floors_interior_tilesandwood_")
            && p.has(f.solidfloor)
        {
            pools.floor = n.clone();
        }
    }
    eprintln!(
        "pool={} floors={} wallN={} wallW={} floor={}",
        pools.pool.len(),
        pools.floors.len(),
        pools.wall_n,
        pools.wall_w,
        pools.floor
    );
    let mut cand = Vec::new();
    for &(wx, wy) in &w.order {
        for z in 0..=2 {
            for y in 0..8 {
                for x in 0..8 {
                    cand.push((wx * 8 + x, wy * 8 + y, z));
                }
            }
        }
    }
    // The answers the harness's hooks gave, from its step lines.
    let mut answers: HashMap<usize, (Option<bool>, Option<f32>)> = HashMap::new();
    for l in oracle.lines() {
        if let Some(rest) = l.strip_prefix("step ") {
            let n: usize = rest.split(' ').next().unwrap().parse().unwrap();
            let powered = rest
                .split(" powered=")
                .nth(1)
                .map(|v| v.starts_with("true"));
            let fluid = rest.split(" fluid=").nth(1).map(|v| {
                f32::from_bits(v.split(' ').next().unwrap().parse::<i32>().unwrap() as u32)
            });
            answers.insert(n, (powered, fluid));
        }
    }
    let steps: usize = std::env::var("STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let full_every: usize = std::env::var("FULL_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let mut rng = Lcg(0x9E3779B97F4A7C15);
    w.rec.borrow_mut().reset();
    drain(&mut w);
    for step in 0..steps {
        let mut p = cand[rng.next(cand.len())];
        let op = rng.next(100);
        if op < 95 && p.2 > 0 && w.sq(p).is_none() && rng.next(4) != 0 {
            p = (p.0, p.1, 0);
        }
        {
            let mut r = w.rec.borrow_mut();
            r.reset();
            let (pw, fl) = answers.get(&step).copied().unwrap_or((None, None));
            r.powered = pw;
            r.fluid = fl;
        }
        let what = do_op(&mut w, &pools, &mut rng, p, op);
        writeln!(w.out, "step {step} {},{},{} {what}", p.0, p.1, p.2).unwrap();
        w.tick();
        drain(&mut w);
        dump_squares(&mut w, p);
        if (step + 1) % full_every == 0 {
            dump_regions(&mut w);
            dump_meta(&mut w);
        }
    }
    dump_regions(&mut w);
    dump_meta(&mut w);
    std::fs::write(out_dir.join("rust_steps.txt"), &w.out).unwrap();
    let mut same = 0;
    let (mut ol, mut rl) = (oracle.lines(), w.out.lines());
    let mut first = None;
    let mut n = 0;
    loop {
        match (ol.next(), rl.next()) {
            (None, None) => break,
            (a, b) => {
                n += 1;
                if a == b {
                    same += 1;
                } else if first.is_none() {
                    first = Some((
                        n,
                        a.unwrap_or("<eof>").to_string(),
                        b.unwrap_or("<eof>").to_string(),
                    ));
                }
            }
        }
    }
    println!(
        "{}: {same}/{n} lines equal (oracle {}, rust {})",
        if w.out == oracle { "MATCH" } else { "DIFF" },
        oracle.lines().count(),
        w.out.lines().count()
    );
    if let Some((n, a, b)) = first {
        println!(
            "first diff at line {n}:\n  java: {}\n  rust: {}",
            &a[..a.len().min(600)],
            &b[..b.len().min(600)]
        );
    }
}
