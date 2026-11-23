//! `zombie.inventory.ItemPickerJava` (Lua's `ItemPicker`): the loot rolls
//! that fill a world container the first time it is explored
//! (`fillContainer`), over the tables of [`crate::item_picker`].
//!
//! Ported: `fillContainer` (rooms, the `all` fallback, `other` and
//! `NO_GENERIC_LOOT_CONTAINERS`, `NoContainerFillRooms`, corpse outfits),
//! `fillContainerType`, `rollItem`, `doRollItem` (rolls, chances, junk,
//! `onlyOne`, a full container stopping the roll), the procedural lists
//! with their `forceFor*` rules and per-room spawn counts,
//! `tryAddItemToContainer` and `ItemSpawner.spawnItem`.
//!
//! Not ported (the spawned item is kept as created): the item
//! post-processing of `doRollItem` (`ItemConfigurator`, stashes, keys and
//! `KeyNamer`, recorded media, auto age, weapon upgrades, drainable uses,
//! conditions, firearms, burnt food, trash/worn/rotten, regions, clothing
//! from washers, `spawnWith`, owner names, the shop flag they read), the
//! bags' own loot (`rollContainerItem`), the zone part of
//! `getCurrentLootMultiplier(square)` and the overlay sprites
//! (`updateOverlaySprite` does nothing).

use crate::bind_item_container::{
    add_item, container_of_value, container_value, has_room_for_weight, is_item_allowed, parent_of, source_grid,
};
use crate::expose::expose_class;
use crate::item_picker::{
    self, actual_spawn_chance, distrib_in_hash_map, find_item, find_item_or_empty, no_generic_loot,
    Distributions, JavaHashMap, LootSettings, PickerContainer, PickerRoom, ProceduralItem,
};
use pz_inventory::shared::{container_of, ContainerRef};
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_sandbox::SandboxOptions;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CLASS: &str = "zombie.inventory.ItemPickerJava";

/// `RandomizedBuildingBase.maximumRoomCount`.
const MAXIMUM_ROOM_COUNT: usize = 500;

thread_local! {
    static SANDBOX: RefCell<Option<Rc<RefCell<SandboxOptions>>>> = const { RefCell::new(None) };
    /// `RoomDef.proceduralSpawnedContainer`, by room id.
    static PROC_SPAWNED: RefCell<HashMap<i64, JavaHashMap>> = RefCell::new(HashMap::new());
}

/// `ItemPickerJava.Parse()` from Lua's globals, then
/// `InitSandboxLootSettings()` from `sb` (kept for the loot multiplier).
pub fn parse(thread: &Thread, sb: Rc<RefCell<SandboxOptions>>) {
    let sm = crate::bind_registry::shared_script_manager();
    let valid = |name: &str| {
        let s = find_item(&sm, name).or_else(|| {
            sm.find_item(name, true)
                .and_then(|s| s.downcast::<pz_scripts::objects::items::Item>())
        });
        s.is_some_and(|s| !s.borrow().obsolete)
    };
    let d = item_picker::parse(thread.globals(), &valid);
    for name in &d.invalid_reported {
        pz_core::debug::DebugType::ItemPicker
            .println(&format!("ignoring invalid ItemPicker item type \"{name}\""));
    }
    item_picker::install(d);
    PROC_SPAWNED.with(|p| p.borrow_mut().clear());
    init_sandbox_loot_settings(sb);
}

/// `ItemPickerJava.InitSandboxLootSettings()`.
pub fn init_sandbox_loot_settings(sb: Rc<RefCell<SandboxOptions>>) {
    let l = LootSettings::from_sandbox(&sb.borrow(), world_age_days(&sb.borrow()));
    item_picker::install_loot_settings(l);
    SANDBOX.with(|s| *s.borrow_mut() = Some(sb));
}

fn world_age_days(sb: &SandboxOptions) -> f64 {
    let gt = crate::bind_climate::shared_game_time();
    let days = gt.borrow().world_age_days_since_begin(sb.time_since_apo());
    days
}

/// The settings of `InitSandboxLootSettings()` with the current
/// `getCurrentLootMultiplier()`.
fn loot() -> LootSettings {
    let mut l = item_picker::loot_settings();
    if let Some(sb) = SANDBOX.with(|s| s.borrow().clone()) {
        let sb = sb.borrow();
        l.loot_multiplier = sb.current_loot_multiplier(world_age_days(&sb));
    }
    l
}

fn rand(max: i32) -> i32 {
    pz_core::random::next(max)
}

fn next_bool(inv: i32) -> bool {
    rand(inv) == 0
}

/// `obj:name(args)`, `Nil` when `obj` is `nil` or has no such method.
fn call(t: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
    if obj.is_nil() {
        return Ok(Value::Nil);
    }
    let f = t.table_get(obj, &Value::str(name))?;
    if f.is_nil() {
        return Ok(Value::Nil);
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

fn class_is(v: &Value, class: &str) -> bool {
    match v {
        Value::Object(o) => o.class_name() == class || o.super_classes().contains(&class),
        _ => false,
    }
}

fn string(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// What a roll needs to know about the container, read once.
struct Target {
    c: ContainerRef,
    kind: String,
    parent: Value,
    square: Value,
    coords: Option<(i32, i32, i32)>,
    room: Option<crate::bind_world::SquareRoom>,
}

impl Target {
    fn new(c: &ContainerRef) -> Self {
        let (kind, parent) = {
            let s = c.borrow();
            (s.kind.clone(), parent_of(&s))
        };
        let square = source_grid(c);
        let coords = crate::bind_world::square_coords(&square);
        let room = coords.and_then(|(x, y, z)| crate::bind_world::square_room_info(x, y, z));
        Self {
            c: c.clone(),
            kind,
            parent,
            square,
            coords,
            room,
        }
    }

    fn is_corpse(&self) -> bool {
        matches!(self.kind.as_str(), "inventorymale" | "inventoryfemale")
    }
}

/// `ItemPickerJava.fillContainer(container, player)`.
pub fn fill_container(t: &mut Thread, d: &Distributions, c: &ContainerRef, player: &Value) -> LuaResult<()> {
    if game_mode(t) == "Tutorial" {
        return Ok(());
    }
    let tg = Target::new(c);
    if tg.square.is_nil() {
        return Ok(());
    }
    let all = d.rooms.get("all");
    if tg.is_corpse() {
        return fill_corpse(t, d, &tg);
    }
    let room_name = tg.room.as_ref().map(|r| r.name.clone());
    match room_name.as_deref().filter(|n| d.rooms.contains_key(*n)) {
        Some(name) => {
            let room = &d.rooms[name];
            let mut dist = room.containers.get(&tg.kind);
            if dist.is_none() && room.containers.contains_key("other") && !no_generic_loot(&tg.kind) {
                dist = room.containers.get("other");
            }
            let mut name = name.to_string();
            if dist.is_none() && room.containers.contains_key("all") {
                dist = room.containers.get("all");
                name = "all".into();
            }
            if dist.is_none() {
                fill_container_type(t, d, all, &tg, &name, player)?;
                return on_fill_container(t, &name, &tg);
            }
            let room_name = tg.room.as_ref().map(|r| r.name.clone()).unwrap_or_default();
            fill_container_type(t, d, Some(room), &tg, &room_name, player)?;
            on_fill_container(t, &room_name, &tg)
        }
        None => {
            let name = room_name.unwrap_or_else(|| "all".into());
            fill_container_type(t, d, all, &tg, &name, player)?;
            on_fill_container(t, &name, &tg)
        }
    }
}

fn on_fill_container(t: &mut Thread, room_name: &str, tg: &Target) -> LuaResult<()> {
    trigger(t, "OnFillContainer", &[Value::str(room_name), Value::str(&tg.kind), container_value(&tg.c)]);
    Ok(())
}

fn trigger(t: &mut Thread, name: &str, args: &[Value]) {
    if let Some(sink) = EVENTS.with(|e| e.borrow().clone()) {
        sink.trigger(t, name, args);
    }
}

thread_local! {
    static EVENTS: RefCell<Option<crate::events::EventManager>> = const { RefCell::new(None) };
}

/// Route `OnFillContainer` to `events`.
pub fn install_event_sink(events: crate::events::EventManager) {
    EVENTS.with(|e| *e.borrow_mut() = Some(events));
}

fn game_mode(t: &Thread) -> String {
    match t.globals().get_str("Core") {
        Value::Table(c) => string(&c.get_str("gameMode")).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The corpse branch of `fillContainer` (`inventorymale` /
/// `inventoryfemale`): the outfit's distribution, then the type's.
fn fill_corpse(t: &mut Thread, d: &Distributions, tg: &Target) -> LuaResult<()> {
    let body = class_is(&tg.parent, "zombie.iso.objects.IsoDeadBody");
    if body && call(t, &tg.parent, "isSkeleton", &[])?.truthy() {
        return Ok(());
    }
    let mut outfit = tg.kind.clone();
    if body {
        outfit = string(&call(t, &tg.parent, "getOutfitName", &[])?).unwrap_or_default();
    }
    let all = d.rooms.get("all");
    let dist = all.and_then(|r| r.containers.get(&format!("Outfit_{outfit}")));
    let default_loot = dist.map_or(true, |c| c.default_inventory_loot);
    if let Some(dist) = dist {
        roll_item(t, d, dist, tg, true, None)?;
    }
    if default_loot {
        if let Some(dist) = all.and_then(|r| r.containers.get(&tg.kind)) {
            roll_item(t, d, dist, tg, true, None)?;
        }
    }
    trigger(t, "OnFillContainer", &[Value::str("Zombie"), Value::str(&tg.kind), container_value(&tg.c)]);
    Ok(())
}

/// `fillContainerType(room, container, roomName, chr)`.
fn fill_container_type(
    t: &mut Thread,
    d: &Distributions,
    room: Option<&PickerRoom>,
    tg: &Target,
    room_name: &str,
    _player: &Value,
) -> LuaResult<()> {
    let do_item_container = !d.no_container_fill_rooms.iter().any(|r| r == room_name);
    let Some(room) = room else {
        return Err(LuaError::runtime("ItemPickerJava.fillContainerType: room is null"));
    };
    if let Some(all) = room.containers.get("all") {
        roll_item(t, d, all, tg, do_item_container, Some(room))?;
    }
    let mut dist = room.containers.get(&tg.kind);
    if dist.is_none() && !no_generic_loot(&tg.kind) {
        dist = room.containers.get("other");
    }
    if let Some(dist) = dist {
        roll_item(t, d, dist, tg, do_item_container, Some(room))?;
    }
    Ok(())
}

/// `rollItem(containerDist, container, doItemContainer, chr, room)`.
fn roll_item(
    t: &mut Thread,
    d: &Distributions,
    dist: &PickerContainer,
    tg: &Target,
    do_item_container: bool,
    room: Option<&PickerRoom>,
) -> LuaResult<()> {
    let player = player_instance(t)?;
    let density = zombie_density_factor(t, dist, tg, &player)?;
    if dist.procedural {
        return roll_procedural(t, d, &dist.procedural_items, tg, density, room);
    }
    if let Some(junk) = &dist.junk {
        do_roll_item(t, junk, tg, density, do_item_container, room, true)?;
    }
    do_roll_item(t, dist, tg, density, do_item_container, room, false)
}

/// `IsoChunk.loadGridSquareIfNeeded` then
/// `LoadGridsquarePerformanceWorkaround.LoadGridsquare` in single player:
/// once per square, the main container of each object with a named sprite
/// (not a floor item) is filled if unexplored and marked explored, then an
/// object without container overlays gets its tile overlay
/// (`TileOverlays.updateTileOverlaySprite`).
/// Not ported: `ItemPickerJava.updateOverlaySprite`.
pub fn load_gridsquare(t: &mut Thread, x: i32, y: i32, z: i32) -> LuaResult<()> {
    // Before `ItemPickerJava.Parse` there are no tables (the game always
    // parses them before a chunk loads).
    let d = item_picker::shared();
    if d.rooms.is_empty() {
        return Ok(());
    }
    let in_use = || LuaError::runtime("LoadGridsquare: IsoGridSquare is in use");
    let Some(square) = crate::bind_world::with_world(|w| {
        w.cell.as_ref().and_then(|cell| cell.get_grid_square(x, y, z))
    }) else {
        return Ok(());
    };
    let objects = {
        let sq = square.try_borrow().map_err(|_| in_use())?;
        if sq.objects.is_empty() {
            return Ok(());
        }
        if sq.overlay_done {
            return Ok(());
        }
        sq.objects.iter().map(|slot| slot.ref_handle()).collect::<Vec<_>>()
    };
    let sprites = crate::bind_sprites::manager()?;
    let square_value = crate::bind_world::square_value(x, y, z);
    // `square.getRoom().getName()` for the overlay choice.
    let room = crate::bind_world::with_world(|w| {
        let meta = w.meta_grid.as_ref()?;
        let sq = square.try_borrow().ok()?;
        sq.room_def(meta).map(|r| r.name.clone())
    });
    for (index, object) in objects.iter().enumerate() {
        let name = {
            let body = object
                .try_borrow()
                .map_err(|_| LuaError::runtime("LoadGridsquare: IsoObject is in use"))?;
            if matches!(body.kind, pz_world::ObjectKind::WorldInventory(_)) {
                continue;
            }
            let sprites = sprites
                .try_borrow()
                .map_err(|_| LuaError::runtime("LoadGridsquare: sprite manager is in use"))?;
            let sprite = body
                .sprite
                .try_read(&sprites)
                .map_err(|e| LuaError::runtime(format!("LoadGridsquare: sprite: {e:?}")))?;
            sprite.and_then(|sprite| sprite.name.clone())
        };
        let Some(name) = name else {
            continue;
        };
        let value = crate::bind_world_object::object_value(object)?;
        crate::bind_world_object::adopt_square(&value, object, &square_value)?;
        let main = object
            .try_borrow()
            .map_err(|_| LuaError::runtime("LoadGridsquare: IsoObject is in use"))?
            .runtime_containers
            .as_ref()
            .and_then(|r| r.main.clone());
        let sq = pz_world::tile_overlays::OverlaySquare { x, y, z, room: room.as_deref() };
        let empty = match main {
            Some(container) => {
                if !container.borrow().explored {
                    let player = player_instance(t)?;
                    fill_container(t, &d, &container, &player)?;
                    container.borrow_mut().explored = true;
                }
                container.borrow().items.is_empty()
            }
            None => false,
        };
        // `checkObject`: an empty container keeps no overlay update.
        if !empty {
            crate::bind_tile_overlays::update_container_overlay_sprite(object, sq)?;
        }
        let container_overlays = crate::bind_tile_overlays::shared_container_overlays();
        let has_container_overlays = container_overlays
            .try_borrow()
            .map_err(|_| LuaError::runtime("LoadGridsquare: ContainerOverlays is in use"))?
            .has_overlays(Some(&name));
        if !has_container_overlays {
            crate::bind_tile_overlays::update_tile_overlay_sprite(&objects, index, &name, sq)?;
        }
    }
    square.try_borrow_mut().map_err(|_| in_use())?.overlay_done = true;
    Ok(())
}

fn player_instance(t: &mut Thread) -> LuaResult<Value> {
    let Value::Table(p) = t.globals().get_str("IsoPlayer") else {
        return Ok(Value::Nil);
    };
    let f = p.get_str("getInstance");
    if f.is_nil() {
        return Ok(Value::Nil);
    }
    t.call1(&f, &[])
}

/// `getZombieDensityFactor(containerDist, container)`.
fn zombie_density_factor(t: &mut Thread, dist: &PickerContainer, tg: &Target, player: &Value) -> LuaResult<f32> {
    let l = item_picker::loot_settings();
    if dist.ignore_zombie_density || l.zombie_pop_loot_effect == 0 {
        return Ok(0.0);
    }
    let pos = if !player.is_nil() {
        let x = pz_kahlua::exec::raw_tonumber(&call(t, player, "getX", &[])?).unwrap_or(0.0) as f32;
        let y = pz_kahlua::exec::raw_tonumber(&call(t, player, "getY", &[])?).unwrap_or(0.0) as f32;
        Some(((x / 8.0).floor() as i32, (y / 8.0).floor() as i32))
    } else {
        tg.coords.map(|(x, y, _)| ((x as f32 / 8.0).floor() as i32, (y as f32 / 8.0).floor() as i32))
    };
    let mut v = 0.0f32;
    if let Some(i) = pos.and_then(|(wx, wy)| crate::bind_world::meta_chunk_zombie_intensity(wx, wy)) {
        let disabled = l.zombies == 6;
        v = item_picker::loot_zombie_intensity(i, disabled, &mut rand);
    }
    Ok(v.min(8.0) * l.zombie_pop_loot_effect as f32)
}

/// `doRollItem(containerDist, container, zombieDensity, chr,
/// doItemContainer, room, isJunk)`.
fn do_roll_item(
    t: &mut Thread,
    dist: &PickerContainer,
    tg: &Target,
    density: f32,
    _do_item_container: bool,
    _room: Option<&PickerRoom>,
    is_junk: bool,
) -> LuaResult<()> {
    let sm = crate::bind_registry::shared_script_manager();
    let l = loot();
    // The washer and dryer draws (the clothing changes they pick are not
    // ported); the shop flag only feeds the post-processing, which is not
    // ported either.
    if class_is(&tg.parent, "zombie.iso.objects.IsoClothingDryer") || tg.kind == "clothingdryer" {
        next_bool(5);
    }
    if class_is(&tg.parent, "zombie.iso.objects.IsoClothingWasher") || tg.kind == "clothingwasher" {
        next_bool(2);
    }
    let rolls = ((dist.rolls as f64 * l.rolls_multiplier) as i32).max(1);
    for _ in 0..rolls {
        for item in &dist.items {
            let (name, _) = find_item_or_empty(&sm, &item.item_name);
            let roll = rand(10000) as f32;
            if roll >= actual_spawn_chance(&sm, &l, item, density, is_junk) {
                continue;
            }
            if try_add_item_to_container(t, tg, &name)?.is_none() {
                return Ok(());
            }
            if dist.only_one {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// `tryAddItemToContainer(container, name, containerDist)`.
fn try_add_item_to_container(
    t: &mut Thread,
    tg: &Target,
    name: &str,
) -> LuaResult<Option<crate::bind_items::ItemRef>> {
    let sm = crate::bind_registry::shared_script_manager();
    let Some(script) = find_item(&sm, name) else {
        return Ok(None);
    };
    let weight = {
        let s = script.borrow();
        if s.obsolete {
            return Ok(None);
        }
        s.actual_weight() * s.count as f32
    };
    if !has_room_for_weight(t, &tg.c, &Value::Nil, weight, weight)? {
        return Ok(None);
    }
    let outer = tg.c.borrow().containing_item.clone();
    if let Some(bag) = outer {
        if let Some(outer) = container_of(&bag) {
            let in_body = class_is(&parent_of(&outer.borrow()), "zombie.iso.objects.IsoDeadBody");
            if !in_body && !has_room_for_weight(t, &outer, &Value::Nil, weight, weight)? {
                return Ok(None);
            }
        }
    }
    let spawned = spawn_item(t, &tg.c, name)?;
    if let (Some(item_ref), Some((x, y, z))) = (&spawned, tg.coords) {
        if let Ok(mut body) = item_ref.try_borrow_mut() {
            if matches!(body.kind, pz_inventory::ItemKind::Key { .. }) {
                crate::bind_key_namer::name_key_at(&mut body, x, y, z);
            }
        }
    }
    Ok(spawned)
}

/// `ItemSpawner.spawnItem(name, container, true)` (the bag roll is not
/// ported).
fn spawn_item(t: &mut Thread, c: &ContainerRef, name: &str) -> LuaResult<Option<crate::bind_items::ItemRef>> {
    let item = crate::bind_items::try_create_item(name, 1.0)?;
    if let Some(i) = &item {
        crate::bind_items::initialise_item(t, i);
    }
    if !is_item_allowed(t, c, item.as_ref())? {
        return Ok(None);
    }
    add_item(t, c, item)
}

/// `rollProceduralItem(list, container, zombieDensity, chr, room)`.
fn roll_procedural(
    t: &mut Thread,
    d: &Distributions,
    list: &[ProceduralItem],
    tg: &Target,
    density: f32,
    room: Option<&PickerRoom>,
) -> LuaResult<()> {
    if tg.square.is_nil() {
        return Ok(());
    }
    let room_def = tg.room.as_ref();
    let has_room = room_def.is_some();
    let room_id = room_def.map(|r| r.id);
    let spawned = |name: &str| -> i32 {
        room_id.map_or(0, |id| {
            PROC_SPAWNED.with(|p| p.borrow().get(&id).and_then(|m| m.get(name)).unwrap_or(0))
        })
    };
    let mut forced = JavaHashMap::default();
    let mut normal = JavaHashMap::default();
    for p in list {
        let weight = if p.weight_chance <= 0 { 1 } else { p.weight_chance };
        if let Some(id) = room_id {
            PROC_SPAWNED.with(|m| {
                let mut m = m.borrow_mut();
                let e = m.entry(id).or_default();
                if e.get(&p.name).is_none() {
                    e.put(&p.name, 0);
                }
            });
        }
        let items_rule = p.force_for_items.as_ref().filter(|_| {
            room_def.is_some_and(|r| r.building_rooms.is_some_and(|n| n <= MAXIMUM_ROOM_COUNT))
        });
        if let (Some(items), Some(r)) = (items_rule, room_def) {
            let z = tg.coords.map_or(0, |c| c.2);
            for x in r.x..r.x2 {
                for y in r.y..r.y2 {
                    if square_sprites(t, x, y, z)?.iter().any(|s| items.contains(s)) {
                        forced.clear();
                        forced.put(&p.name, -1);
                    }
                }
            }
        } else if let Some(tiles) = &p.force_for_tiles {
            if let Some((x, y, z)) = tg.coords {
                if square_sprites(t, x, y, z)?.iter().any(|s| tiles.contains(s)) {
                    forced.clear();
                    forced.put(&p.name, -1);
                }
            }
        } else if let Some(zones) = &p.force_for_zones {
            if let Some((x, y, _)) = tg.coords {
                for (ty, name) in zones_at(t, x, y)? {
                    if room_id.is_some() && spawned(&p.name) >= p.max {
                        continue;
                    }
                    if zones.contains(&ty) || zones.contains(&name) {
                        forced.clear();
                        forced.put(&p.name, -1);
                        break;
                    }
                }
            }
        } else if let (Some(rooms), true) = (&p.force_for_rooms, has_room) {
            let building = call(t, &tg.square, "getBuilding", &[])?;
            for r in rooms {
                if !call(t, &building, "getRandomRoom", &[Value::str(r)])?.is_nil() {
                    forced.clear();
                    forced.put(&p.name, -1);
                    break;
                }
            }
        }
        if p.force_for_items.is_none()
            && p.force_for_zones.is_none()
            && p.force_for_tiles.is_none()
            && p.force_for_rooms.is_none()
        {
            if has_room && p.min == 1 && spawned(&p.name) == 0 {
                forced.put(&p.name, weight);
            } else if !has_room || spawned(&p.name) < p.max {
                normal.put(&p.name, weight);
            }
        }
    }
    let pick = if !forced.is_empty() {
        distrib_in_hash_map(&forced, &mut rand)
    } else if !normal.is_empty() {
        distrib_in_hash_map(&normal, &mut rand)
    } else {
        None
    };
    let Some(pick) = pick else { return Ok(()) };
    let Some(dist) = d.procedural.get(&pick) else {
        return Ok(());
    };
    if let Some(junk) = &dist.junk {
        do_roll_item(t, junk, tg, density, true, room, true)?;
    }
    do_roll_item(t, dist, tg, density, true, room, false)?;
    if let Some(id) = room_id {
        PROC_SPAWNED.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.entry(id).or_default();
            let n = e.get(&pick).unwrap_or(0);
            e.put(&pick, n + 1);
        });
    }
    Ok(())
}

/// The sprite names of the objects on the square at `(x, y, z)`.
fn square_sprites(t: &mut Thread, x: i32, y: i32, z: i32) -> LuaResult<Vec<String>> {
    let sq = crate::bind_world::square_value(x, y, z);
    let objects = call(t, &sq, "getObjects", &[])?;
    let n = pz_kahlua::exec::raw_tonumber(&call(t, &objects, "size", &[])?).unwrap_or(0.0) as i32;
    let mut out = Vec::new();
    for i in 0..n {
        let o = call(t, &objects, "get", &[Value::Num(i as f64)])?;
        let s = call(t, &o, "getSprite", &[])?;
        if let Some(name) = string(&call(t, &s, "getName", &[])?) {
            out.push(name);
        }
    }
    Ok(out)
}

/// `IsoWorld.instance.metaGrid.getZonesAt(x, y, 0)`: each zone's type and
/// name (empty when the Lua binding has no zones).
fn zones_at(t: &mut Thread, x: i32, y: i32) -> LuaResult<Vec<(String, String)>> {
    let world = match t.globals().get_str("getWorld") {
        f @ (Value::Native(_) | Value::Closure(_)) => t.call1(&f, &[])?,
        _ => Value::Nil,
    };
    let grid = call(t, &world, "getMetaGrid", &[])?;
    let zones = call(t, &grid, "getZonesAt", &[Value::Num(x as f64), Value::Num(y as f64), Value::Num(0.0)])?;
    let n = pz_kahlua::exec::raw_tonumber(&call(t, &zones, "size", &[])?).unwrap_or(0.0) as i32;
    let mut out = Vec::new();
    for i in 0..n {
        let z = call(t, &zones, "get", &[Value::Num(i as f64)])?;
        let ty = string(&call(t, &z, "getType", &[])?).unwrap_or_default();
        let name = string(&call(t, &z, "getName", &[])?).unwrap_or_default();
        out.push((ty, name));
    }
    Ok(out)
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.static_fn("fillContainer", |c| {
        let Some(container) = container_of_value(&c.arg(0)) else {
            return Err(LuaError::runtime("ItemPickerJava.fillContainer: container is null"));
        };
        let player = c.arg(1);
        let d = item_picker::shared();
        fill_container(c.thread, &d, &container, &player)?;
        Ok(0)
    });
    // STUB: the overlay sprites (`doOverlaySprite`) are not ported.
    b.static_fn("updateOverlaySprite", |_| Ok(0));
    b.static_fn("getLootModifier", |c| {
        let name = string(&c.arg(0)).unwrap_or_default();
        let sm = crate::bind_registry::shared_script_manager();
        let v = item_picker::loot_modifier(&sm, &item_picker::loot_settings(), &name);
        Ok(c.push(v as f64))
    });
}

#[cfg(test)]
#[path = "bind_item_picker/tests.rs"]
mod tests;
