//! `zombie.iso.BrokenFences`: the breakable fence sprites
//! `media/lua/server/Items/BrokenFences.lua` registers
//! (`addBrokenTiles`: each unbroken sprite's fully broken, left-broken and
//! right-broken replacements; `addDebrisTiles`: the debris sprite for each
//! side), and the decisions `destroyFence` / `damageAdjacent` /
//! `updateSprite` / `addDebrisObject` / `addItems` make with them.
//!
//! The square and object mutations (replacing the sprite, adding the debris
//! object and items, the recalculations, the sound) are the engine's; these
//! functions return what to do, drawing `Rand` (`RandStandard`) where the
//! original does.

use pz_pathfind::mover::ObjectRef;
use pz_world::chunk::CHUNK_WIDTH;
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::{
    IsoCell, IsoDirections, IsoGridSquare, IsoMetaGrid, IsoObjectRef, IsoSpriteManager, ObjectSlot,
};
use std::collections::HashMap;

use crate::{StatesEngine, StatesWorldAdapter};

/// `RandStandard.INSTANCE` as the fence code draws it.
pub trait FenceRand {
    /// `Rand.Next(max)`.
    fn next(&mut self, max: i32) -> i32;
    /// `Rand.Next(min, max)` for floats.
    fn next_float(&mut self, min: f32, max: f32) -> f32;
}

/// One `BrokenFences.Tile`. A list is `None` when its table is missing or
/// empty (`tableToTiles`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrokenTile {
    pub self_: Option<Vec<String>>,
    pub left: Option<Vec<String>>,
    pub right: Option<Vec<String>>,
    pub debris_n: Option<Vec<String>>,
    pub debris_s: Option<Vec<String>>,
    pub debris_w: Option<Vec<String>>,
    pub debris_e: Option<Vec<String>>,
}

/// `Tile.pickRandom(choices)`: `PZArrayUtil.pickRandom` (`Rand.Next(size)`).
fn pick_random(choices: &Option<Vec<String>>, rand: &mut dyn FenceRand) -> Option<String> {
    let c = choices.as_ref()?;
    if c.is_empty() {
        return None;
    }
    Some(c[rand.next(c.len() as i32) as usize].clone())
}

/// `addDebrisTiles()` with a key `addBrokenTiles` did not register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownTile;

impl std::fmt::Display for UnknownTile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("addDebrisTiles() with unknown tile")
    }
}

impl std::error::Error for UnknownTile {}

/// What `damageAdjacent` does to the breakable fence next to a destroyed
/// one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdjacentDamage {
    /// Nothing (a north-west corner seen from the south or east).
    None,
    /// `destroyFence(obj, dirBreak)`: it was already broken on the other
    /// side.
    Destroy,
    /// `updateSprite(obj, breakLeft, breakRight)`.
    Damage { left: bool, right: bool },
}

/// One debris item `addItems` drops: the script type and the square
/// offsets.
#[derive(Clone, Debug, PartialEq)]
pub struct DebrisItem {
    pub item_type: &'static str,
    pub x: f32,
    pub y: f32,
}

/// `BrokenFences.getInstance()`.
#[derive(Clone, Debug, Default)]
pub struct BrokenFences {
    /// The `Tile` objects, shared by the maps as in Java.
    pub tiles: Vec<BrokenTile>,
    pub unbroken_map: HashMap<String, usize>,
    pub broken_left_map: HashMap<String, usize>,
    pub broken_right_map: HashMap<String, usize>,
    pub all_map: HashMap<String, usize>,
}

impl BrokenFences {
    /// `addBrokenTiles(tiles)`: one `(key, self, left, right)` per
    /// non-`VERSION` key, in table order.
    pub fn add_broken_tiles(&mut self, defs: Vec<(String, BrokenTile)>) {
        for (key, tile) in defs {
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let i = self.tiles.len();
            for s in tile.left.iter().flatten() {
                self.broken_left_map.insert(s.clone(), i);
            }
            for s in tile.right.iter().flatten() {
                self.broken_right_map.insert(s.clone(), i);
            }
            self.tiles.push(BrokenTile {
                debris_n: None,
                debris_s: None,
                debris_w: None,
                debris_e: None,
                ..tile
            });
            self.unbroken_map.insert(key, i);
        }
        for m in [
            &self.unbroken_map,
            &self.broken_left_map,
            &self.broken_right_map,
        ] {
            for (k, &v) in m {
                self.all_map.insert(k.clone(), v);
            }
        }
    }

    /// `addDebrisTiles(tiles)`: `(key, north, south, west, east)`; the
    /// tiles before an unknown key keep their debris.
    #[allow(clippy::type_complexity)]
    pub fn add_debris_tiles(
        &mut self,
        defs: Vec<(String, [Option<Vec<String>>; 4])>,
    ) -> Result<(), UnknownTile> {
        for (key, [n, s, w, e]) in defs {
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let &i = self.unbroken_map.get(&key).ok_or(UnknownTile)?;
            let t = &mut self.tiles[i];
            t.debris_n = n;
            t.debris_s = s;
            t.debris_w = w;
            t.debris_e = e;
        }
        Ok(())
    }

    fn tile(&self, sprite_name: &str) -> Option<&BrokenTile> {
        self.all_map.get(sprite_name).map(|&i| &self.tiles[i])
    }

    /// `isUnbroken(obj)` for the object's sprite name.
    pub fn is_unbroken(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.unbroken_map.contains_key(n))
    }

    /// `isBrokenLeft(obj)`.
    pub fn is_broken_left(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.broken_left_map.contains_key(n))
    }

    /// `isBrokenRight(obj)`.
    pub fn is_broken_right(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.broken_right_map.contains_key(n))
    }

    /// `isBreakableObject(obj)` / `isBreakableSprite(name)`.
    pub fn is_breakable_sprite(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.all_map.contains_key(n))
    }

    /// `updateSprite(obj, brokenLeft, brokenRight)`'s new sprite name
    /// (`None`: the object keeps its sprite).
    pub fn update_sprite_name(
        &self,
        sprite_name: Option<&str>,
        broken_left: bool,
        broken_right: bool,
        rand: &mut dyn FenceRand,
    ) -> Option<String> {
        let tile = self.tile(sprite_name?)?;
        if broken_left && broken_right {
            pick_random(&tile.self_, rand)
        } else if broken_left {
            pick_random(&tile.left, rand)
        } else if broken_right {
            pick_random(&tile.right, rand)
        } else {
            None
        }
    }

    /// `getBreakableObject(square, north)`: the index of the first object
    /// (`(sprite name, collideN, collideW)` in square order) that is
    /// breakable and collides on that side.
    pub fn get_breakable_object(
        &self,
        objects: &[(Option<&str>, bool, bool)],
        north: bool,
    ) -> Option<usize> {
        objects.iter().position(|&(name, n, w)| {
            self.is_breakable_sprite(name) && ((north && n) || (!north && w))
        })
    }

    /// `damageAdjacent(square, dirAdjacent, dirBreak)` for the breakable
    /// object found on the adjacent square (`is_nw`: it collides north and
    /// west).
    pub fn adjacent_damage(
        &self,
        sprite_name: Option<&str>,
        is_nw: bool,
        dir_adjacent: IsoDirections,
    ) -> AdjacentDamage {
        use IsoDirections as D;
        let left = matches!(dir_adjacent, D::N | D::E);
        let right = matches!(dir_adjacent, D::S | D::W);
        if is_nw && matches!(dir_adjacent, D::S | D::E) {
            return AdjacentDamage::None;
        }
        if left && self.is_broken_right(sprite_name) {
            return AdjacentDamage::Destroy;
        }
        if right && self.is_broken_left(sprite_name) {
            return AdjacentDamage::Destroy;
        }
        AdjacentDamage::Damage { left, right }
    }

    /// The `damageAdjacent` calls `destroyFence` makes after destroying a
    /// fence that collides north / west: `(dirAdjacent, north)` where
    /// `north` is the side `getBreakableObject` looks for on that square.
    pub fn destroy_adjacent_dirs(north: bool, west: bool) -> Vec<(IsoDirections, bool)> {
        use IsoDirections as D;
        let dirs: &[D] = if north && west {
            &[D::S, D::E]
        } else if north {
            &[D::W, D::E]
        } else if west {
            &[D::N, D::S]
        } else {
            &[]
        };
        dirs.iter()
            .map(|&d| (d, matches!(d, D::W | D::E)))
            .collect()
    }

    /// `addDebrisObject(obj, dir)`'s debris sprite and whether it goes on
    /// the adjacent square in `dir` (north and west) instead of the
    /// fence's. `Err` for a direction other than N/S/W/E
    /// (`IllegalArgumentException("invalid direction")`).
    pub fn debris_sprite(
        &self,
        sprite_name: Option<&str>,
        dir: IsoDirections,
        rand: &mut dyn FenceRand,
    ) -> Result<Option<(String, bool)>, &'static str> {
        let Some(tile) = sprite_name.and_then(|n| self.tile(n)) else {
            return Ok(None);
        };
        let (list, adjacent) = match dir {
            IsoDirections::N => (&tile.debris_n, true),
            IsoDirections::S => (&tile.debris_s, false),
            IsoDirections::W => (&tile.debris_w, true),
            IsoDirections::E => (&tile.debris_e, false),
            _ => return Err("invalid direction"),
        };
        Ok(pick_random(list, rand).map(|s| (s, adjacent)))
    }

    /// `addItems(obj, square)`: the items dropped for the object's
    /// `Material` / `Material2` / `Material3` properties.
    pub fn add_items(materials: [Option<&str>; 3], rand: &mut dyn FenceRand) -> Vec<DebrisItem> {
        let has = |m: &str| materials.iter().any(|x| *x == Some(m));
        let mut out = Vec::new();
        let mut add = |t: &'static str, rand: &mut dyn FenceRand| {
            let x = rand.next_float(0.0, 0.5);
            let y = rand.next_float(0.0, 0.5);
            out.push(DebrisItem { item_type: t, x, y });
        };
        if has("Wood") {
            add("Base.UnusableWood", rand);
            if rand.next(5) == 0 {
                add("Base.UnusableWood", rand);
            }
        }
        if has("MetalBars") && rand.next(2) == 0 {
            add("Base.MetalBar", rand);
        }
        if has("MetalWire") && rand.next(3) == 0 {
            add("Base.Wire", rand);
        }
        if has("MetalPlates") && rand.next(2) == 0 {
            add("Base.SheetMetal", rand);
            add("Base.NutsBolts", rand);
        }
        if has("MetalScrap") && rand.next(2) == 0 {
            add("Base.SteelChunk", rand);
            add("Base.SteelPiece", rand);
        }
        if has("Nails") && rand.next(2) == 0 {
            add("Base.Nails", rand);
        }
        if has("Screws") && rand.next(2) == 0 {
            add("Base.Screws", rand);
        }
        out
    }

    /// `Reset()`. The `Tile` objects stay reachable in Java only through the
    /// maps, so they go too.
    pub fn reset(&mut self) {
        self.unbroken_map.clear();
        self.broken_left_map.clear();
        self.broken_right_map.clear();
        self.all_map.clear();
        self.tiles.clear();
    }
}

/// Host callbacks used by [`destroy_fence_in_cell`] and friends for random
/// draws, break sounds, debris item drops, and geometry/path/region invalidation.
pub trait FenceWorldHooks: FenceRand {
    fn play_break_sound(&mut self, _sound: &str, _obj: ObjectRef) {}

    #[allow(clippy::too_many_arguments)]
    fn spawn_debris_item(
        &mut self,
        _cell: &mut IsoCell,
        _meta: &IsoMetaGrid,
        _sprites: &mut IsoSpriteManager,
        _sq: (i32, i32, i32),
        _item_type: &str,
        _x: f32,
        _y: f32,
    ) {
    }

    fn geometry_changed(
        &mut self,
        _cell: &IsoCell,
        _meta: &IsoMetaGrid,
        _sprites: &IsoSpriteManager,
        _pos: (i32, i32, i32),
    ) {
    }
}

fn sprite_name_of(obj: &IsoObject, sprites: &IsoSpriteManager) -> Option<String> {
    obj.sprite
        .try_read(sprites)
        .ok()
        .flatten()
        .and_then(|s| s.name.clone())
}

fn obj_has_flag(obj: &IsoObject, sprites: &IsoSpriteManager, flag_name: &str) -> bool {
    let Some(flag) = sprites.tables.flag_from_string(flag_name) else {
        return false;
    };
    obj.sprite
        .try_read(sprites)
        .ok()
        .flatten()
        .is_some_and(|s| s.properties.has(flag))
}

fn obj_prop(obj: &IsoObject, sprites: &IsoSpriteManager, key: &str) -> Option<String> {
    obj.sprite
        .try_read(sprites)
        .ok()
        .flatten()
        .and_then(|s| s.properties.get(&sprites.alias, key).map(str::to_string))
}

fn sprite_named(sprites: &mut IsoSpriteManager, name: &str) -> usize {
    match sprites.named_map.get(name) {
        Some(&idx) => idx,
        None => {
            let idx = sprites.add_sprite_named(name);
            sprites.sprites[idx].name = Some(name.to_string());
            idx
        }
    }
}

fn replace_sprite_in_cell(cell: &mut IsoCell, object: &IsoObjectRef, sprite: usize) {
    let (old, native_class) = {
        let mut object = object.borrow_mut();
        let native_class = object.kind.native_class_name();
        let old = std::mem::replace(&mut object.sprite, Some(sprite).into());
        (old, native_class)
    };
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_payload(Box::new(old));
    cell.retire_components(retired);
    pz_world::object_slot::observe_object(object, native_class);
}

fn find_object_on_cell(cell: &IsoCell, r: ObjectRef) -> Option<(usize, IsoObjectRef)> {
    let sq_owner = cell.get_grid_square(r.x, r.y, r.z)?;
    let sq = sq_owner.borrow();
    let idx = sq.object_index(r.id)?;
    Some((idx, sq.objects[idx].ref_handle()))
}

/// `BrokenFences.destroyFence` break sound selection: `IsoThumpable.getBreakSound()`,
/// else `"ZombieThumpWoodCollapse"` when `ThumpSound == "ZombieThumpWood"`,
/// else `"BreakObject"`.
pub fn fence_break_sound(obj: &IsoObject, sprites: &IsoSpriteManager) -> String {
    if let ObjectKind::Thumpable(t) = &obj.kind {
        if let Some(s) = t.break_sound.as_deref().filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    if obj_prop(obj, sprites, "ThumpSound").as_deref() == Some("ZombieThumpWood") {
        return "ZombieThumpWoodCollapse".to_string();
    }
    "BreakObject".to_string()
}

/// `[Material, Material2, Material3]` properties of `obj` for [`BrokenFences::add_items`].
pub fn object_materials(obj: &IsoObject, sprites: &IsoSpriteManager) -> [Option<String>; 3] {
    [
        obj_prop(obj, sprites, "Material"),
        obj_prop(obj, sprites, "Material2"),
        obj_prop(obj, sprites, "Material3"),
    ]
}

/// `BrokenFences.getBreakableObject(square, north)`.
pub fn breakable_object_on_square(
    fences: &BrokenFences,
    sq: &IsoGridSquare,
    sprites: &IsoSpriteManager,
    north: bool,
) -> Option<IsoObjectRef> {
    let names: Vec<(Option<String>, bool, bool)> = sq
        .objects
        .iter()
        .map(|slot| {
            let o = slot.borrow();
            (
                sprite_name_of(&o, sprites),
                obj_has_flag(&o, sprites, "collideN"),
                obj_has_flag(&o, sprites, "collideW"),
            )
        })
        .collect();
    let refs: Vec<(Option<&str>, bool, bool)> = names
        .iter()
        .map(|(n, cn, cw)| (n.as_deref(), *cn, *cw))
        .collect();
    let idx = fences.get_breakable_object(&refs, north)?;
    Some(sq.objects[idx].ref_handle())
}

/// `BrokenFences.addItems(obj, square)` on `cell`.
pub fn add_debris_items_in_cell(
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    sprites: &mut IsoSpriteManager,
    sq_pos: (i32, i32, i32),
    materials: [Option<String>; 3],
    hooks: &mut dyn FenceWorldHooks,
) {
    let mat_refs = [
        materials[0].as_deref(),
        materials[1].as_deref(),
        materials[2].as_deref(),
    ];
    let items = BrokenFences::add_items(mat_refs, hooks);
    for item in items {
        hooks.spawn_debris_item(cell, meta, sprites, sq_pos, item.item_type, item.x, item.y);
    }
}

/// `BrokenFences.updateSprite(obj, brokenLeft, brokenRight)` on `cell`.
#[allow(clippy::too_many_arguments)]
pub fn update_fence_sprite_in_cell(
    fences: &BrokenFences,
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    sprites: &mut IsoSpriteManager,
    r: ObjectRef,
    broken_left: bool,
    broken_right: bool,
    hooks: &mut dyn FenceWorldHooks,
) -> bool {
    let Some((_, obj_ref)) = find_object_on_cell(cell, r) else {
        return false;
    };
    let name = {
        let o = obj_ref.borrow();
        sprite_name_of(&o, sprites)
    };
    if !fences.is_breakable_sprite(name.as_deref()) {
        return false;
    }
    let Some(new_name) = fences.update_sprite_name(name.as_deref(), broken_left, broken_right, hooks)
    else {
        return false;
    };
    let new_idx = sprite_named(sprites, &new_name);
    replace_sprite_in_cell(cell, &obj_ref, new_idx);
    cell.recalc_all_with_neighbours(r.x, r.y, r.z, meta, sprites);
    hooks.geometry_changed(cell, meta, sprites, (r.x, r.y, r.z));
    true
}

/// `BrokenFences.setDestroyed(obj)` on `cell`.
pub fn set_fence_destroyed_in_cell(
    fences: &BrokenFences,
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    sprites: &mut IsoSpriteManager,
    r: ObjectRef,
    hooks: &mut dyn FenceWorldHooks,
) -> bool {
    let Some((_, obj_ref)) = find_object_on_cell(cell, r) else {
        return false;
    };
    obj_ref.borrow_mut().attached_anim_sprite = None;
    // `square.removeBlood(false, false)`: strip floor blood splats on this tile.
    let wx = r.x.div_euclid(CHUNK_WIDTH);
    let wy = r.y.div_euclid(CHUNK_WIDTH);
    if let Some(chunk) = cell.chunks.get_mut(&(wx, wy)) {
        chunk.saved.floor_blood_splats.retain(|sp| {
            sp.x.floor() as i32 != r.x || sp.y.floor() as i32 != r.y || sp.z.floor() as i32 != r.z
        });
    }
    update_fence_sprite_in_cell(fences, cell, meta, sprites, r, true, true, hooks)
}

/// `BrokenFences.setDamagedLeft(obj)` on `cell`.
pub fn set_fence_damaged_left_in_cell(
    fences: &BrokenFences,
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    sprites: &mut IsoSpriteManager,
    r: ObjectRef,
    hooks: &mut dyn FenceWorldHooks,
) -> bool {
    update_fence_sprite_in_cell(fences, cell, meta, sprites, r, true, false, hooks)
}

/// `BrokenFences.setDamagedRight(obj)` on `cell`.
pub fn set_fence_damaged_right_in_cell(
    fences: &BrokenFences,
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    sprites: &mut IsoSpriteManager,
    r: ObjectRef,
    hooks: &mut dyn FenceWorldHooks,
) -> bool {
    update_fence_sprite_in_cell(fences, cell, meta, sprites, r, false, true, hooks)
}

/// `BrokenFences.destroyFence(obj, dir)` on `cell`.
pub fn destroy_fence_in_cell(
    fences: &BrokenFences,
    cell: &mut IsoCell,
    meta: &IsoMetaGrid,
    sprites: &mut IsoSpriteManager,
    mut r: ObjectRef,
    dir: IsoDirections,
    hooks: &mut dyn FenceWorldHooks,
) -> Result<bool, &'static str> {
    let Some((obj_index, obj_ref)) = find_object_on_cell(cell, r) else {
        return Ok(false);
    };
    let (sprite_name, sound, collide_n, collide_w, is_thumpable, materials, raw_sprite) = {
        let o = obj_ref.borrow();
        let sprite_name = sprite_name_of(&o, sprites);
        if !fences.is_breakable_sprite(sprite_name.as_deref()) {
            return Ok(false);
        }
        let sound = fence_break_sound(&o, sprites);
        let collide_n = obj_has_flag(&o, sprites, "collideN");
        let collide_w = obj_has_flag(&o, sprites, "collideW");
        let is_thumpable = matches!(o.kind, ObjectKind::Thumpable(_));
        let materials = object_materials(&o, sprites);
        let raw_sprite = o.sprite.clone();
        (
            sprite_name,
            sound,
            collide_n,
            collide_w,
            is_thumpable,
            materials,
            raw_sprite,
        )
    };
    hooks.play_break_sound(&sound, r);

    // Replace an `IsoThumpable` fence in place with a plain `IsoObject` at the
    // same square index so the destroyed stub is no longer thumpable.
    if is_thumpable {
        if let Some(sq_owner) = cell.get_grid_square_mut(r.x, r.y, r.z) {
            let mut sq = sq_owner.borrow_mut();
            if let Some(i) = sq.object_index(r.id) {
                let new_id = sq.new_object_id();
                let mut replacement = IsoObject::new(new_id, ObjectKind::Object, None);
                replacement.sprite = raw_sprite;
                let old_slot = sq.remove_object_slot(i);
                sq.special_objects.retain(|&id| id != r.id);
                sq.objects.insert(i, ObjectSlot::new(replacement));
                drop(sq);
                let mut retired = pz_entity::RetiredComponents::new();
                retired.push_payload(Box::new(old_slot));
                cell.retire_components(retired);
                r.id = new_id;
            }
        }
    }

    // `addDebrisObject(obj, dir)`.
    if let Some((debris_name, adjacent)) = fences.debris_sprite(sprite_name.as_deref(), dir, hooks)? {
        let debris_pos = if adjacent {
            let (dx, dy) = dir.delta();
            (r.x + dx, r.y + dy, r.z)
        } else {
            (r.x, r.y, r.z)
        };
        let can_place = cell
            .get_grid_square(debris_pos.0, debris_pos.1, debris_pos.2)
            .is_some_and(|sq| sq.borrow().treat_as_solid_floor(sprites));
        if can_place {
            let debris_idx = sprite_named(sprites, &debris_name);
            if let Some(sq_owner) = cell.get_grid_square_mut(debris_pos.0, debris_pos.1, debris_pos.2) {
                let mut sq = sq_owner.borrow_mut();
                let id = sq.new_object_id();
                let debris_obj = IsoObject::new(id, ObjectKind::Object, Some(debris_idx));
                let insert_at = if debris_pos == (r.x, r.y, r.z) {
                    obj_index.min(sq.objects.len())
                } else {
                    sq.objects.len()
                };
                sq.objects.insert(insert_at, ObjectSlot::new(debris_obj));
                sq.burnt_out = false;
            }
            add_debris_items_in_cell(cell, meta, sprites, debris_pos, materials, hooks);
            cell.recalc_all_with_neighbours(debris_pos.0, debris_pos.1, debris_pos.2, meta, sprites);
            hooks.geometry_changed(cell, meta, sprites, debris_pos);
        }
    }

    // `setDestroyed(obj)`.
    set_fence_destroyed_in_cell(fences, cell, meta, sprites, r, hooks);

    // `damageAdjacent(square, dirAdjacent, dirBreak)`.
    for (dir_adj, north) in BrokenFences::destroy_adjacent_dirs(collide_n, collide_w) {
        let (dx, dy) = dir_adj.delta();
        let adj_pos = (r.x + dx, r.y + dy, r.z);
        let adj_hit = cell
            .get_grid_square(adj_pos.0, adj_pos.1, adj_pos.2)
            .and_then(|sq| {
                let sq = sq.borrow();
                let obj = breakable_object_on_square(fences, &sq, sprites, north)?;
                let o = obj.borrow();
                let name = sprite_name_of(&o, sprites);
                let is_nw = obj_has_flag(&o, sprites, "collideN")
                    && obj_has_flag(&o, sprites, "collideW");
                Some((
                    ObjectRef {
                        x: adj_pos.0,
                        y: adj_pos.1,
                        z: adj_pos.2,
                        id: o.id,
                    },
                    name,
                    is_nw,
                ))
            });
        let Some((adj_ref, adj_name, is_nw)) = adj_hit else {
            continue;
        };
        match fences.adjacent_damage(adj_name.as_deref(), is_nw, dir_adj) {
            AdjacentDamage::None => {}
            AdjacentDamage::Destroy => {
                let _ = destroy_fence_in_cell(fences, cell, meta, sprites, adj_ref, dir, hooks);
            }
            AdjacentDamage::Damage { left, right } => {
                update_fence_sprite_in_cell(fences, cell, meta, sprites, adj_ref, left, right, hooks);
            }
        }
    }

    cell.recalc_all_with_neighbours(r.x, r.y, r.z, meta, sprites);
    hooks.geometry_changed(cell, meta, sprites, (r.x, r.y, r.z));
    Ok(true)
}

struct EngineFenceHooks<'a>(&'a mut dyn StatesEngine);

impl FenceRand for EngineFenceHooks<'_> {
    fn next(&mut self, max: i32) -> i32 {
        self.0.rand().next(max)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        min + self.0.rand().next_float() * (max - min)
    }
}

impl FenceWorldHooks for EngineFenceHooks<'_> {
    fn play_break_sound(&mut self, sound: &str, obj: ObjectRef) {
        self.0.play_world_sound(sound, (obj.x, obj.y, obj.z));
    }

    fn spawn_debris_item(
        &mut self,
        cell: &mut IsoCell,
        meta: &IsoMetaGrid,
        sprites: &mut IsoSpriteManager,
        sq: (i32, i32, i32),
        item_type: &str,
        x: f32,
        y: f32,
    ) {
        self.0
            .spawn_broken_fence_item(cell, meta, sprites, sq, item_type, x, y);
    }

    fn geometry_changed(
        &mut self,
        cell: &IsoCell,
        meta: &IsoMetaGrid,
        sprites: &IsoSpriteManager,
        pos: (i32, i32, i32),
    ) {
        self.0.geometry_changed(cell, meta, sprites, pos);
    }
}

impl StatesWorldAdapter<'_> {
    /// `BrokenFences.getInstance().destroyFence(obj, dir)`.
    /// Uses [`StatesEngine::broken_fences`] when installed, falling back to
    /// [`StatesEngine::destroy_fence`] for mock test engines.
    pub fn destroy_fence(&mut self, r: ObjectRef, dir: IsoDirections) {
        if let Some(bf) = self.engine.broken_fences() {
            let fences = bf.borrow().clone();
            let mut hooks = EngineFenceHooks(self.engine);
            let _ = destroy_fence_in_cell(
                &fences,
                self.cell,
                self.meta,
                self.sprites,
                r,
                dir,
                &mut hooks,
            );
            return;
        }
        self.engine.destroy_fence(self.cell, r, dir);
    }

    /// `BrokenFences.getInstance().addItems(obj, square)` for removed/collapsed
    /// fence pieces, delegating to both [`StatesEngine::broken_fence_items`]
    /// and [`StatesEngine::spawn_broken_fence_item`] when `BrokenFences` is live.
    pub(crate) fn drop_broken_fence_items(&mut self, sq: (i32, i32, i32), obj: &IsoObjectRef) {
        self.engine.broken_fence_items(sq, obj);
        if self.engine.broken_fences().is_some() {
            let materials = object_materials(&obj.borrow(), self.sprites);
            let mut hooks = EngineFenceHooks(self.engine);
            add_debris_items_in_cell(self.cell, self.meta, self.sprites, sq, materials, &mut hooks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Seq(Vec<i32>);

    impl FenceRand for Seq {
        fn next(&mut self, max: i32) -> i32 {
            self.0.remove(0).min(max - 1)
        }
        fn next_float(&mut self, min: f32, _max: f32) -> f32 {
            min
        }
    }

    fn v(s: &[&str]) -> Option<Vec<String>> {
        (!s.is_empty()).then(|| s.iter().map(|s| s.to_string()).collect())
    }

    fn fences() -> BrokenFences {
        let mut b = BrokenFences::default();
        b.add_broken_tiles(vec![
            ("VERSION".into(), BrokenTile::default()),
            (
                "f_1".into(),
                BrokenTile {
                    self_: v(&["d_1"]),
                    left: v(&["d_2"]),
                    right: v(&["d_0", "d_9"]),
                    ..Default::default()
                },
            ),
        ]);
        b
    }

    #[test]
    fn registry_and_sprites() {
        let mut b = fences();
        assert!(b.is_unbroken(Some("f_1")) && !b.is_unbroken(Some("d_2")));
        assert!(b.is_broken_left(Some("d_2")) && b.is_broken_right(Some("d_9")));
        assert!(b.is_breakable_sprite(Some("d_0")) && !b.is_breakable_sprite(Some("d_1")));
        assert!(!b.is_breakable_sprite(None));
        // From a right-broken piece, breaking the left side picks `left` of
        // the same tile.
        let mut r = Seq(vec![0]);
        assert_eq!(
            b.update_sprite_name(Some("d_9"), true, false, &mut r)
                .as_deref(),
            Some("d_2")
        );
        let mut r = Seq(vec![1]);
        assert_eq!(
            b.update_sprite_name(Some("f_1"), false, true, &mut r)
                .as_deref(),
            Some("d_9")
        );
        assert_eq!(
            b.update_sprite_name(Some("f_1"), false, false, &mut r),
            None
        );
        assert_eq!(
            b.add_debris_tiles(vec![("nope".into(), Default::default())]),
            Err(UnknownTile)
        );
        b.add_debris_tiles(vec![(
            "f_1".into(),
            [v(&["n_0"]), v(&["s_0"]), None, v(&["e_0"])],
        )])
        .unwrap();
        let mut r = Seq(vec![0, 0]);
        assert_eq!(
            b.debris_sprite(Some("f_1"), IsoDirections::N, &mut r),
            Ok(Some(("n_0".into(), true)))
        );
        assert_eq!(
            b.debris_sprite(Some("f_1"), IsoDirections::W, &mut r),
            Ok(None)
        );
        assert!(b
            .debris_sprite(Some("f_1"), IsoDirections::NW, &mut r)
            .is_err());
        b.reset();
        assert!(b.all_map.is_empty());
    }

    #[test]
    fn adjacent_rules() {
        let b = fences();
        assert_eq!(
            b.adjacent_damage(Some("f_1"), true, IsoDirections::S),
            AdjacentDamage::None
        );
        assert_eq!(
            b.adjacent_damage(Some("d_9"), false, IsoDirections::N),
            AdjacentDamage::Destroy
        );
        assert_eq!(
            b.adjacent_damage(Some("d_2"), false, IsoDirections::W),
            AdjacentDamage::Destroy
        );
        assert_eq!(
            b.adjacent_damage(Some("f_1"), false, IsoDirections::E),
            AdjacentDamage::Damage {
                left: true,
                right: false
            }
        );
        assert_eq!(
            BrokenFences::destroy_adjacent_dirs(true, false),
            vec![(IsoDirections::W, true), (IsoDirections::E, true)]
        );
        assert_eq!(
            BrokenFences::destroy_adjacent_dirs(false, true),
            vec![(IsoDirections::N, false), (IsoDirections::S, false)]
        );
        let objs = [
            (Some("x"), true, false),
            (Some("f_1"), false, true),
            (Some("d_2"), true, false),
        ];
        assert_eq!(b.get_breakable_object(&objs, true), Some(2));
        assert_eq!(b.get_breakable_object(&objs, false), Some(1));
    }

    #[test]
    fn items() {
        // Wood: one plank, a second on Next(5) == 0; MetalWire on Next(3) == 0.
        let mut r = Seq(vec![0, 1]);
        let it = BrokenFences::add_items([Some("Wood"), None, Some("MetalWire")], &mut r);
        let types: Vec<_> = it.iter().map(|i| i.item_type).collect();
        assert_eq!(types, ["Base.UnusableWood", "Base.UnusableWood"]);
    }

    #[test]
    fn destroy_fence_in_cell_updates_sprite_places_debris_damages_adjacent_and_drops_items() {
        use pz_world::chunk::{FloorBloodSplat, IsoChunk};
        use pz_world::objects::IsoThumpable;
        use pz_world::SquareSlot;

        let mut sprites = IsoSpriteManager::new(crate::tests::tables());
        let mut alias_map = pz_world::sprite::PropertyValueMap::default();
        for k in [
            "WindowN",
            "WindowW",
            "DoorWallN",
            "DoorWallW",
            "WallSE",
            "tree",
            "lightR",
            "lightG",
            "lightB",
            "ItemHeight",
            "Surface",
            "Bush",
            "forceLocked",
        ] {
            alias_map.entry(k);
        }
        alias_map.entry("Material").push("Wood".into());
        alias_map.entry("ThumpSound").push("ZombieThumpWood".into());
        sprites.generate_alias_map(alias_map, &[]).unwrap();
        let floor_idx = sprite_named(&mut sprites, "floors_0");
        let fence_idx = sprite_named(&mut sprites, "f_1");
        let broken_left_idx = sprite_named(&mut sprites, "d_2");
        let broken_right_idx = sprite_named(&mut sprites, "d_0");
        let _broken_self_idx = sprite_named(&mut sprites, "d_1");
        let _debris_s_idx = sprite_named(&mut sprites, "deb_s");

        let solidfloor = sprites.tables.flag_from_string("solidfloor").unwrap();
        let collide_n = sprites.tables.flag_from_string("collideN").unwrap();
        sprites.sprites[floor_idx].properties.set_flag(solidfloor);
        for idx in [fence_idx, broken_left_idx, broken_right_idx] {
            let mut p = sprites.sprites[idx].properties.clone();
            p.set_flag(collide_n);
            p.set(&sprites.tables, &sprites.alias, "Material", "Wood", false);
            p.set(
                &sprites.tables,
                &sprites.alias,
                "ThumpSound",
                "ZombieThumpWood",
                false,
            );
            sprites.sprites[idx].properties = p;
        }

        let mut cell = IsoCell::default();
        let mut ch = IsoChunk::new(0, 0);
        ch.saved.floor_blood_splats.push(FloorBloodSplat {
            x: 5.2,
            y: 5.4,
            z: 0.0,
            kind: 1,
            world_age: 1.0,
            index: 0,
        });

        let mut make_sq = |x: i32, y: i32, fence_sprite: Option<usize>, thumpable: bool| -> ObjectRef {
            let mut sq = IsoGridSquare::new(x, y, 0);
            let fid = sq.new_object_id();
            sq.objects
                .push(ObjectSlot::new(IsoObject::new(fid, ObjectKind::Object, Some(floor_idx))));
            let out_id = if let Some(s) = fence_sprite {
                let id = sq.new_object_id();
                let kind = if thumpable {
                    ObjectKind::Thumpable(Box::new(IsoThumpable::default()))
                } else {
                    ObjectKind::Object
                };
                sq.objects.push(ObjectSlot::new(IsoObject::new(id, kind, Some(s))));
                id
            } else {
                fid
            };
            sq.recalc_properties(&sprites);
            ch.set_square(x, y, 0, Some(SquareSlot::new(sq)));
            ObjectRef { x, y, z: 0, id: out_id }
        };

        // Row of 3 north-edge fences at x=4 (already broken-left -> losing east neighbor cascades to Destroy!),
        // x=5 (target thumpable fence), x=6 (unbroken -> losing west neighbor becomes broken-left `d_2`).
        let left_ref = make_sq(4, 5, Some(broken_left_idx), false);
        let center_ref = make_sq(5, 5, Some(fence_idx), true);
        let right_ref = make_sq(6, 5, Some(fence_idx), false);
        cell.chunks.insert((0, 0), ch);

        let mut bf = fences();
        bf.add_debris_tiles(vec![(
            "f_1".into(),
            [v(&["deb_n"]), v(&["deb_s"]), v(&["deb_w"]), v(&["deb_e"])],
        )])
        .unwrap();

        struct RecHooks {
            sounds: Vec<String>,
            items: Vec<((i32, i32, i32), String)>,
            geo: Vec<(i32, i32, i32)>,
        }
        impl FenceRand for RecHooks {
            fn next(&mut self, max: i32) -> i32 {
                if max <= 1 { 0 } else { 1 }
            }
            fn next_float(&mut self, min: f32, _max: f32) -> f32 {
                min
            }
        }
        impl FenceWorldHooks for RecHooks {
            fn play_break_sound(&mut self, sound: &str, _obj: ObjectRef) {
                self.sounds.push(sound.to_string());
            }
            fn spawn_debris_item(
                &mut self,
                _cell: &mut IsoCell,
                _meta: &IsoMetaGrid,
                _sprites: &mut IsoSpriteManager,
                sq: (i32, i32, i32),
                item_type: &str,
                _x: f32,
                _y: f32,
            ) {
                self.items.push((sq, item_type.to_string()));
            }
            fn geometry_changed(
                &mut self,
                _cell: &IsoCell,
                _meta: &IsoMetaGrid,
                _sprites: &IsoSpriteManager,
                pos: (i32, i32, i32),
            ) {
                self.geo.push(pos);
            }
        }

        let meta = IsoMetaGrid::empty();
        let mut hooks = RecHooks {
            sounds: Vec::new(),
            items: Vec::new(),
            geo: Vec::new(),
        };
        let destroyed = destroy_fence_in_cell(
            &bf,
            &mut cell,
            &meta,
            &mut sprites,
            center_ref,
            IsoDirections::S,
            &mut hooks,
        )
        .unwrap();
        assert!(destroyed);

        // Center square: thumpable replaced by plain IsoObject with sprite `d_1`, plus `deb_s` debris, blood cleared.
        let center_names: Vec<Option<String>> = cell
            .get_grid_square(5, 5, 0)
            .unwrap()
            .borrow()
            .objects
            .iter()
            .map(|slot| sprite_name_of(&slot.borrow(), &sprites))
            .collect();
        assert!(center_names.contains(&Some("d_1".into())), "center names={center_names:?}");
        assert!(center_names.contains(&Some("deb_s".into())), "center names={center_names:?}");
        assert!(
            cell.get_grid_square(5, 5, 0)
                .unwrap()
                .borrow()
                .objects
                .iter()
                .all(|slot| !matches!(slot.borrow().kind, ObjectKind::Thumpable(_))),
            "destroyed thumpable fence should become plain IsoObject"
        );
        assert!(
            cell.chunks[&(0, 0)].saved.floor_blood_splats.is_empty(),
            "setDestroyed should clear floor blood splats on the fence square"
        );

        // Left square (x=4): was broken-right (`d_0`), damaged on east -> cascaded to `d_1`!
        let left_sprite = cell
            .get_grid_square(4, 5, 0)
            .unwrap()
            .borrow()
            .objects
            .iter()
            .find(|slot| slot.borrow().id == left_ref.id)
            .and_then(|slot| sprite_name_of(&slot.borrow(), &sprites));
        assert_eq!(left_sprite.as_deref(), Some("d_1"));

        // Right square (x=6): was unbroken `f_1`, damaged on left -> became `d_2`!
        let right_sprite = cell
            .get_grid_square(6, 5, 0)
            .unwrap()
            .borrow()
            .objects
            .iter()
            .find(|slot| slot.borrow().id == right_ref.id)
            .and_then(|slot| sprite_name_of(&slot.borrow(), &sprites));
        assert_eq!(right_sprite.as_deref(), Some("d_2"));

        assert!(hooks.sounds.iter().any(|s| s == "ZombieThumpWoodCollapse"));
        assert!(hooks.items.iter().any(|(sq, t)| *sq == (5, 5, 0) && t == "Base.UnusableWood"));
        assert!(hooks.geo.contains(&(5, 5, 0)));
    }
}
