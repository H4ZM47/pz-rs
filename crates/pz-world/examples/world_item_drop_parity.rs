//! Items dropping to the floor ([`pz_inventory::floor_drop`] over
//! [`pz_world::square_mutation::FloorDropEnv`]) against the original's
//! `setWornItem` `forceDropTooHeavy` block (`hasRoomFor`,
//! `getSolidFloorAt`, `Remove`, `Rand.Next(0.1f, 0.9f)`, `getApparentZ`,
//! `AddWorldInventoryItem`) and direct `AddWorldInventoryItem` calls, run
//! by a local harness on the same chunk files with the same pseudo-random
//! steps and seeded `Rand` (no game data is part of this repository).
//!
//! ```text
//! world_item_drop_parity <game dir> <sprite tables> <save tables> <save dir> <oracle drops.txt> <out dir>
//! ```
//!
//! Each step picks a square of the `WINDOW`-wide window (levels 0 to 3), an
//! item type and a mode, creates the item (`Rand` seeded `7000 + step`)
//! and up to four `Base.Log`s into a player's inventory, then (`Rand`
//! seeded `1000 + step`) either runs the force drop from that square or
//! puts the item on the square with fixed offsets. Writes `rust_drops.txt`
//! (the world object, the item, the square's object lists, the next `Rand`
//! draw) and prints the first difference.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use pz_inventory::context::NoOwner;
use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{create_item_by_type, parse_item_names, ScriptItems};
use pz_inventory::floor_drop::{self, DropCharacter};
use pz_inventory::shared::{self, ContainerRef, ItemRef};
use pz_inventory::{InventoryItem, ItemContext, ItemTemplate, SharedContainer};
use pz_world::isoregion::IsoRegions;
use pz_world::objects::ObjectName;
use pz_world::save::chunk_file::{apply_loaded_window_sprites, chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::square_mutation::{
    DoorTables, FloorDropEnv, MutationEnv, NoMutationHooks, RenderSquareHooks, SquarePos,
    WorldItemPlaced,
};
use pz_world::{GameRand, IsoCell, IsoMetaGrid, IsoSpriteManager, ObjectKind, SpriteTables};

/// `java.util.Random`, shared by the world and the item factory.
#[derive(Clone)]
struct JavaRandom(Rc<RefCell<u64>>);

impl JavaRandom {
    fn seed(&self, seed: i64) {
        *self.0.borrow_mut() = (seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1);
    }
    fn bits(&self, bits: u32) -> i32 {
        let mut s = self.0.borrow_mut();
        *s = (s.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (*s >> (48 - bits)) as i64 as i32
    }
    fn int(&self, bound: i32) -> i32 {
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
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
        self.int(bound)
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

impl pz_inventory::component::FactoryRng for JavaRandom {
    fn next_int(&mut self, bound: i32) -> i32 {
        self.int(bound)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        self.next_float_range(min, max)
    }
}

/// `PerformanceSettings.fboRenderChunk` is on in the harness.
struct Fbo;
impl RenderSquareHooks for Fbo {
    fn fbo_render_chunk(&self) -> bool {
        true
    }
}

/// The game state `getUnequippedWeight` reads.
struct Ctx {
    scripts: HashMap<String, Rc<ItemTemplate>>,
}

impl ItemContext for Ctx {
    fn world_age_hours(&self) -> f64 {
        0.0
    }
    fn multiplier(&self) -> f32 {
        1.0
    }
    fn rain_intensity(&self) -> f32 {
        0.0
    }
    fn is_raining(&self) -> bool {
        false
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        1.0
    }
    fn basic_mask_defense(&self) -> f32 {
        0.0
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn script(&self, name: &str) -> Option<Rc<ItemTemplate>> {
        let full = if name.contains('.') {
            name.to_string()
        } else {
            format!("Base.{name}")
        };
        self.scripts.get(&full).cloned()
    }
}

/// A player standing on `square` holding `inv` (nothing worn or in hand).
struct Player<'a> {
    inv: ContainerRef,
    square: Option<SquarePos>,
    ctx: &'a Ctx,
}

impl DropCharacter for Player<'_> {
    fn inventory(&self) -> ContainerRef {
        self.inv.clone()
    }
    fn is_player(&self) -> bool {
        true
    }
    fn is_in_vehicle(&self) -> bool {
        false
    }
    fn current_square(&self) -> Option<SquarePos> {
        self.square
    }
    /// `getInventoryWeight()`: every item is unequipped.
    fn capacity_weight(&self) -> f32 {
        let mut total = 0.0f32;
        for i in &self.inv.borrow().items {
            total += i.borrow().get_unequipped_weight(self.ctx, &NoOwner);
        }
        total
    }
    fn unequipped_weight(&self, item: &ItemRef) -> f32 {
        item.borrow().get_unequipped_weight(self.ctx, &NoOwner)
    }
    fn primary_hand_item(&self) -> Option<ItemRef> {
        None
    }
    fn secondary_hand_item(&self) -> Option<ItemRef> {
        None
    }
    fn set_primary_hand_item(&mut self, _: Option<&ItemRef>) {}
    fn set_secondary_hand_item(&mut self, _: Option<&ItemRef>) {}
}

/// The harness's step generator.
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

fn h(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

const TYPES: [&str; 18] = [
    "Base.Hammer",
    "Base.Plank",
    "Base.Bag_Schoolbag",
    "Base.Bag_DuffelBag",
    "Base.WaterBottle",
    "Base.Key1",
    "Base.IronOre",
    "Base.LargeMeteorite",
    "Base.BlockAnvil",
    "Base.Apple",
    "Base.Tshirt_DefaultTEXTURE",
    "Base.Jacket_Padded",
    "Base.Sledgehammer",
    "Base.Pot",
    "Base.Log",
    "Base.Rope",
    "Base.Nails",
    "Base.PetrolCan",
];

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
    let names = parse_item_names(
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
    for &(wx, wy) in &order {
        apply_loaded_window_sprites(&cell.chunks[&(wx, wy)], &mut sprites);
    }
    // Start from the harness's loaded objects where the chunk-file loader
    // differs: `init.txt` next to the oracle steps.
    let init = Path::new(&a[5]).with_file_name("init.txt");
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
            let name = o.sprite.and_then(|s| sprites.sprites[s].name.clone());
            match java.get(i) {
                Some(&(class, spr)) if class == o.kind.class_name() => {
                    if name.as_deref().unwrap_or("null") != spr {
                        if let Some(&s) = sprites.named_map.get(spr) {
                            o.sprite = Some(s);
                        }
                    }
                    i += 1;
                }
                _ => {
                    drop(o);
                    let id = sq.borrow_mut().remove_object_slot(i).borrow().id;
                    sq.borrow_mut().world_objects.retain(|&w| w != id);
                    sq.borrow_mut().special_objects.retain(|&w| w != id);
                }
            }
        }
    }
    for &(wx, wy) in &order {
        let ch = cell.chunks.get_mut(&(wx, wy)).unwrap();
        for z in ch.min_level..=ch.max_level {
            for y in 0..8 {
                for x in 0..8 {
                    if let Some(sq) = ch.get_grid_square_mut(x, y, z) {
                        sq.borrow_mut().recalc_properties(&sprites);
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
    let mut regions = IsoRegions::new(win);
    let mut scripts = HashMap::new();
    for rc in pz_scripts::objects::items::get_all_items(&sm) {
        if let Some(t) = ItemTemplate::from_script_ref(&sm, &rc) {
            scripts.insert(rc.borrow().full_name(), t);
        }
    }
    let ctx = Ctx { scripts };
    let rand = JavaRandom(Rc::new(RefCell::new(0)));
    *registry.rand.borrow_mut() = Some(Box::new(rand.clone()));
    let display = |full: &str| names.get(full).cloned();
    // `getUnequippedWeight()` of an item on the floor: its fluid is in the
    // world object.
    let fluid_id = pz_entity::ComponentType::FluidContainer.id();
    let weight = |o: &pz_world::IsoObject, i: &InventoryItem| {
        let mut i = i.clone();
        if let Some(b) = o.components.iter().find(|b| b.type_id == fluid_id) {
            i.components.push(b.clone());
            i.component_cache = None;
        }
        i.get_unequipped_weight(&ctx, &NoOwner)
    };

    let mut cand = Vec::new();
    for &(wx, wy) in &order {
        for z in 0..=3 {
            for y in 0..8 {
                for x in 0..8 {
                    cand.push((wx * 8 + x, wy * 8 + y, z));
                }
            }
        }
    }
    let steps = oracle.lines().count();
    let mut lcg = Lcg(0x9E3779B97F4A7C15);
    let mut out = String::new();
    let (mut world_rand, mut hooks, mut path, mut fbo) =
        (rand.clone(), NoMutationHooks, NoMutationHooks, Fbo);
    for step in 0..steps {
        let p = cand[lcg.next(cand.len())];
        let ty = TYPES[lcg.next(TYPES.len())];
        let mode = lcg.next(10);
        let n_fill = lcg.next(5);
        let ox = if lcg.next(3) == 0 {
            0.0
        } else {
            0.125 * (1 + lcg.next(7)) as f32
        };
        let oy = if lcg.next(3) == 0 {
            0.0
        } else {
            0.125 * (1 + lcg.next(7)) as f32
        };
        let oh = if lcg.next(2) == 0 { 0.0 } else { 0.25f32 };
        let zrot = lcg.next(3);
        let mut s = format!("step {step} {},{},{} {ty} mode={mode}", p.0, p.1, p.2);
        rand.seed(7000 + step as i64);
        let Some(item) = create_item_by_type(&registry, &sm, ty, 1.0, true, &display) else {
            writeln!(out, "{s} noitem").unwrap();
            continue;
        };
        let item = Rc::new(RefCell::new(item));
        if zrot == 1 {
            item.borrow_mut().world_z_rotation = 45.0;
        }
        let inv = SharedContainer::default().into_ref();
        shared::add_item(&inv, &item);
        for _ in 0..n_fill {
            let log = create_item_by_type(&registry, &sm, "Base.Log", 1.0, true, &display).unwrap();
            shared::add_item(&inv, &Rc::new(RefCell::new(log)));
        }
        let have_sq = cell.get_grid_square(p.0, p.1, p.2).is_some();
        let mut player = Player {
            inv: inv.clone(),
            square: have_sq.then_some(p),
            ctx: &ctx,
        };
        rand.seed(1000 + step as i64);
        let mut env = MutationEnv {
            cell: &mut cell,
            meta: &meta,
            sprites: &mut sprites,
            regions: &mut regions,
            rand: &mut world_rand,
            doors: &doors,
            world: &mut hooks,
            pathfind: &mut path,
            render: &mut fbo,
            num_players: 1,
        };
        let mut drop_sq = None;
        let (mut dx, mut dy, mut dz) = (0.0, 0.0, 0.0);
        if mode < 7 {
            let room = floor_drop::has_room_for(&mut player, &item);
            write!(
                s,
                " cw={} w={} room={room}",
                h(player.capacity_weight()),
                h(player.unequipped_weight(&item))
            )
            .unwrap();
            if !room {
                if player.square.is_none() {
                    s.push_str(" nosquare");
                } else {
                    let mut floor = FloorDropEnv::new(&mut env, 0.0);
                    match floor_drop::force_drop_too_heavy(
                        &mut player,
                        &mut floor,
                        true,
                        Some(&item),
                    ) {
                        None => s.push_str(" nofloor"),
                        Some(d) => {
                            drop_sq = Some(d.square);
                            (dx, dy, dz) = (d.x, d.y, d.z);
                        }
                    }
                }
            }
        } else if !have_sq {
            s.push_str(" nosq");
        } else {
            floor_drop::remove(&mut player, &item);
            drop_sq = Some(p);
            (dx, dy, dz) = (ox, oy, oh);
            let placed = env.add_world_inventory_item(p, &item, ox, oy, oh, 0.0);
            assert!(matches!(placed, WorldItemPlaced::Object(_)), "{placed:?}");
        }
        write!(s, " inv={}", inv.borrow().items.len()).unwrap();
        if let Some(dp) = drop_sq {
            let sq = cell.get_grid_square(dp.0, dp.1, dp.2).unwrap();
            let id = *sq.borrow().world_objects.last().unwrap();
            let idx = sq.borrow().object_index(id).unwrap();
            let o_owner = sq.borrow().objects[idx].ref_handle();
            let o = o_owner.borrow();
            let ObjectKind::WorldInventory(wi) = &o.kind else {
                panic!()
            };
            let wi = wi.borrow();
            let i = item.borrow();
            write!(
                s,
                " at {},{},{} d={},{},{} off={},{},{}",
                dp.0,
                dp.1,
                dp.2,
                h(dx),
                h(dy),
                h(dz),
                h(wi.xoff),
                h(wi.yoff),
                h(wi.zoff)
            )
            .unwrap();
            let name = match &o.name {
                Some(ObjectName::Str(n)) => n.clone(),
                other => format!("{other:?}"),
            };
            write!(
                s,
                " name={name} key={} rot={},{},{} cont={} type={}",
                o.key_id,
                h(i.world_x_rotation),
                h(i.world_y_rotation),
                h(i.world_z_rotation),
                if shared::container_of(&item).is_some() {
                    "set"
                } else {
                    "null"
                },
                i.get_full_type()
            )
            .unwrap();
            let item_comps = !i.components.is_empty() || i.component_cache.is_some();
            write!(
                s,
                " objFluid={} itemFluid={item_comps} idx={idx}/{} world={} special={} floorW={}",
                o.has_components,
                sq.borrow().objects.len(),
                sq.borrow().world_objects.len(),
                sq.borrow().special_objects.len(),
                h(sq.borrow().get_total_weight_of_items_on_floor(weight))
            )
            .unwrap();
        }
        write!(s, " next={}", rand.int(1000000)).unwrap();
        writeln!(out, "{s}").unwrap();
    }
    std::fs::create_dir_all(out_dir).unwrap();
    std::fs::write(out_dir.join("rust_drops.txt"), &out).unwrap();
    // `getSolidFloorAt` from every candidate, `getApparentZ` on stairs and
    // slopes (`apparent.txt` next to the oracle steps).
    let pts = [
        (0.0, 0.0),
        (0.5, 0.5),
        (0.25, 0.75),
        (1.0, 0.3),
        (-0.5, 1.5),
        (0.9, 0.1),
    ];
    let mut app = String::new();
    for &p in &cand {
        let f = cell.get_solid_floor_at(p.0, p.1, p.2, &sprites);
        write!(
            app,
            "{},{},{} floor={}",
            p.0,
            p.1,
            p.2,
            f.map_or("none".to_string(), |f| f.2.to_string())
        )
        .unwrap();
        if let Some(q) = cell.get_grid_square(p.0, p.1, p.2) {
            if q.borrow().has_stairs(&sprites) || q.borrow().has_sloped_surface() {
                app.push_str(" z=");
                for (dx, dy) in pts {
                    write!(
                        app,
                        "{};",
                        h(q.borrow().get_apparent_z(dx, dy, &sprites, true))
                    )
                    .unwrap();
                }
            }
        }
        app.push('\n');
    }
    std::fs::write(out_dir.join("rust_apparent.txt"), &app).unwrap();
    let java_app = std::fs::read_to_string(Path::new(&a[5]).with_file_name("apparent.txt"))
        .unwrap_or_default();
    let (mut same_a, mut diff_a, mut sloped) = (0, 0, 0);
    for (j, r) in java_app.lines().zip(app.lines()) {
        if r.contains(" z=") {
            sloped += 1;
        }
        if j == r {
            same_a += 1;
        } else {
            if diff_a == 0 {
                println!("apparent java: {j}\napparent rust: {r}");
            }
            diff_a += 1;
        }
    }
    println!(
        "apparent lines={} same={same_a} diff={diff_a} stairs/slopes={sloped}",
        app.lines().count()
    );
    let (mut same, mut diff, mut first) = (0, 0, None);
    for (j, r) in oracle.lines().zip(out.lines()) {
        if j == r {
            same += 1;
        } else {
            diff += 1;
            first.get_or_insert((j.to_string(), r.to_string()));
        }
    }
    let placed = out.lines().filter(|l| l.contains(" at ")).count();
    println!("steps={steps} same={same} diff={diff} placed={placed}");
    if let Some((j, r)) = first {
        println!("java: {j}\nrust: {r}");
    }
}
