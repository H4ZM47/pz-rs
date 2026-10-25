//! `zombie.iso.BentFences`: the fences zombies bend, collapse or smash.
//!
//! [`BentFences`] holds the tile lists Lua registers
//! (`media/lua/server/Items/BentFences.lua` calls `addFenceTiles`), as
//! [`FenceTileDef`]s, and the sandbox values it reads. The engine owns one
//! and hands it out through [`crate::StatesEngine::bent_fences`]; the
//! adapter runs the object code (`bendFence`, `swapTiles`, `collapse`,
//! `smashFence`, `checkDamageHoppableFence`, ...) on the cell.
//!
//! Characters hit by a collapsing fence are the engine's
//! ([`crate::StatesEngine::fence_collapsed`]); `BrokenFences` (breakable
//! fences, their items) is the engine's too.

use std::collections::{HashMap, HashSet};

use pz_pathfind::mover::ObjectRef;
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::square::EdgeFacing;
use pz_world::{IsoDirections, IsoGridSquare};

use crate::StatesWorldAdapter;

/// One `tiles` table entry Lua passes to `addFenceTiles` (absent keys:
/// `None`, or `-1` / `false` as `rawgetInt` / `rawgetBool` read them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FenceTileDef {
    /// `dir` (`IsoDirections.valueOf`).
    pub dir: Option<IsoDirections>,
    pub health: i32,
    pub collapsed_offset: i32,
    pub collapsed_size_x: i32,
    pub collapsed_size_y: i32,
    pub do_smash: bool,
    /// `stages`: `{stage, tiles}` in table order.
    pub stages: Option<Vec<(i32, Vec<String>)>>,
    pub collapsed: Option<Vec<String>>,
    pub debris: Option<Vec<String>>,
}

/// `BentFences.Entry`.
#[derive(Clone, Debug, PartialEq)]
pub struct BentFenceEntry {
    pub dir: Option<IsoDirections>,
    pub health: i32,
    pub stages: HashMap<i32, Vec<String>>,
    pub collapsed: Vec<String>,
    pub debris: Vec<String>,
    pub length: i32,
    pub collapsed_offset: i32,
    pub collapsed_size_x: i32,
    pub collapsed_size_y: i32,
    pub do_smash: bool,
}

impl BentFenceEntry {
    fn is_north(&self) -> bool {
        matches!(self.dir, Some(IsoDirections::N | IsoDirections::S))
    }

    /// `getFacingDirection()`.
    pub fn facing(&self) -> EdgeFacing {
        EdgeFacing::of_north(self.is_north())
    }

    fn stage(&self, i: i32) -> Option<&Vec<String>> {
        self.stages.get(&i)
    }

    fn stage_count(&self) -> i32 {
        self.stages.len() as i32
    }
}

/// `BentFences.ThumpData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumpData {
    pub total_thumpers: i32,
    pub bend_stage: i32,
    pub direction_to_bend: Option<IsoDirections>,
    pub direction_bent: Option<IsoDirections>,
    pub health: i32,
    pub stages: i32,
    pub thumpers_to_damage: i32,
    pub damage_modifier: f32,
    pub damage_multiplier: f32,
}

impl Default for ThumpData {
    fn default() -> Self {
        ThumpData {
            total_thumpers: 0,
            bend_stage: 0,
            direction_to_bend: None,
            direction_bent: None,
            health: 100,
            stages: 0,
            thumpers_to_damage: 100,
            damage_modifier: 0.1,
            damage_multiplier: 1.0,
        }
    }
}

/// `BentFences.getInstance()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BentFences {
    pub entries: Vec<BentFenceEntry>,
    pub collapsed_tiles: HashSet<String>,
    pub debris_tiles: HashSet<String>,
    /// `fenceMap`: entry indices by sprite name.
    pub fence_map: HashMap<String, Vec<usize>>,
    /// `SandboxOptions.lore.fenceThumpersRequired`.
    pub thumpers_required: i32,
    /// `SandboxOptions.lore.fenceDamageMultiplier`.
    pub damage_multiplier: f32,
}

impl BentFences {
    /// An empty registry with the sandbox values.
    pub fn new(thumpers_required: i32, damage_multiplier: f32) -> Self {
        BentFences {
            thumpers_required,
            damage_multiplier,
            ..Default::default()
        }
    }

    /// `isEnabled()`.
    pub fn is_enabled(&self) -> bool {
        self.thumpers_required >= 1
    }

    /// `addFenceTiles(version, tiles)`.
    pub fn add_fence_tiles(&mut self, _version: i32, tiles: &[FenceTileDef]) {
        for d in tiles {
            let mut e = BentFenceEntry {
                dir: d.dir,
                health: if d.health != -1 { d.health } else { 100 },
                stages: HashMap::new(),
                collapsed: Vec::new(),
                debris: Vec::new(),
                length: -1,
                collapsed_offset: d.collapsed_offset,
                collapsed_size_x: d.collapsed_size_x,
                collapsed_size_y: d.collapsed_size_y,
                do_smash: d.do_smash,
            };
            let Some(stages) = &d.stages else { continue };
            for (stage, tiles) in stages {
                e.stages.insert(*stage, tiles.clone());
            }
            if let Some(c) = &d.collapsed {
                e.collapsed.extend(c.iter().cloned());
                if e.collapsed_size_x * e.collapsed_size_y != e.collapsed.len() as i32 {
                    continue;
                }
                self.collapsed_tiles.extend(e.collapsed.iter().cloned());
            }
            if let Some(dbr) = &d.debris {
                e.debris.extend(dbr.iter().cloned());
                self.debris_tiles.extend(dbr.iter().cloned());
            }
            if e.stages.is_empty() {
                continue;
            }
            e.length = e.stage(0).map_or(0, |s| s.len() as i32);
            let idx = self.entries.len();
            let n = e.stage_count();
            let names: Vec<String> = (0..n)
                .filter_map(|i| e.stage(i))
                .flatten()
                .cloned()
                .collect();
            self.entries.push(e);
            for name in names {
                self.fence_map.entry(name).or_default().push(idx);
            }
        }
    }
}

/// A sprite-name view of an object: the square and the object.
type Found<'s> = (&'s IsoGridSquare, &'s IsoObject);

impl StatesWorldAdapter<'_> {
    fn obj_sprite_name<'s>(&'s self, o: &IsoObject) -> Option<&'s str> {
        o.sprite
            .and_then(|s| self.sprites.sprites[s].name.as_deref())
    }

    /// `getBendStage(obj, entry)`.
    fn bf_bend_stage(&self, o: &IsoObject, e: Option<&BentFenceEntry>) -> i32 {
        let (Some(name), Some(e)) = (self.obj_sprite_name(o), e) else {
            return -1;
        };
        for i in 0..e.stage_count() {
            if e.stage(i).is_some_and(|t| t.iter().any(|x| x == name)) {
                return i;
            }
        }
        -1
    }

    /// `getObjectForEntry(square, tiles, index)`.
    fn bf_object_for_entry<'s>(
        &'s self,
        sq: &'s IsoGridSquare,
        tiles: &[String],
        index: usize,
    ) -> Option<&'s IsoObject> {
        sq.objects
            .iter()
            .find(|o| self.obj_sprite_name(o) == Some(tiles[index].as_str()))
    }

    /// The square `i - index` along the fence from `sq`.
    fn bf_span_square(
        &self,
        sq: &IsoGridSquare,
        e: &BentFenceEntry,
        i: i32,
        index: i32,
    ) -> Option<&IsoGridSquare> {
        let x = sq.x + if e.is_north() { i - index } else { 0 };
        let y = sq.y + if e.is_north() { 0 } else { i - index };
        self.cell.get_grid_square(x, y, sq.z)
    }

    /// `isValidSpan(tiles, obj, entry, index)`.
    fn bf_valid_span(
        &self,
        tiles: &[String],
        sq: &IsoGridSquare,
        e: &BentFenceEntry,
        index: i32,
    ) -> bool {
        for i in 0..tiles.len() as i32 {
            let Some(s) = self.bf_span_square(sq, e, i, index) else {
                return false;
            };
            if index == i || self.bf_object_for_entry(s, tiles, i as usize).is_some() {
                continue;
            }
            return false;
        }
        true
    }

    /// `getTileIndex(tiles, obj, entry, forceFirst)`.
    fn bf_tile_index(
        &self,
        tiles: &[String],
        (sq, o): Found<'_>,
        e: &BentFenceEntry,
        force_first: bool,
    ) -> i32 {
        let mut index = -1;
        let mut last = -1;
        let center = tiles.len() as i32 / 2;
        if let Some(name) = self.obj_sprite_name(o) {
            for (i, t) in tiles.iter().enumerate() {
                let i = i as i32;
                if t != name || !self.bf_valid_span(tiles, sq, e, i) {
                    continue;
                }
                if force_first || i == center {
                    return i;
                }
                last = index;
                index = i;
            }
        }
        if self.bf_bend_stage(o, Some(e)) == 0 {
            if last == -1 {
                index
            } else {
                last
            }
        } else {
            index
        }
    }

    /// `isValidObject(obj, entry)`.
    fn bf_valid_object(&self, f: Found<'_>, e: &BentFenceEntry) -> bool {
        let stage = self.bf_bend_stage(f.1, Some(e));
        if stage == -1 {
            return false;
        }
        let Some(tiles) = e.stage(stage) else {
            return false;
        };
        self.bf_tile_index(tiles, f, e, false) != -1
    }

    fn is_stairs_object(&self, o: &IsoObject) -> bool {
        let ty = &self.sprites.tables.ty;
        o.sprite.is_some_and(|s| {
            let t = self.sprites.sprites[s].tile_type;
            [
                ty.stairsTN,
                ty.stairsMN,
                ty.stairsBN,
                ty.stairsTW,
                ty.stairsMW,
                ty.stairsBW,
            ]
            .contains(&t)
        })
    }

    fn obj_flag(&self, o: &IsoObject, name: &str) -> bool {
        self.env().obj_has_flag(o, name)
    }

    fn is_wall_n(&self, o: &IsoObject) -> bool {
        self.obj_flag(o, "WallN") || self.obj_flag(o, "WallNTrans")
    }

    fn is_wall_w(&self, o: &IsoObject) -> bool {
        self.obj_flag(o, "WallW") || self.obj_flag(o, "WallWTrans")
    }

    /// `IsoObject.isWall()` (`sprite != null && isWallN() || isWallW()`).
    fn is_wall_obj(&self, o: &IsoObject) -> bool {
        o.sprite.is_some() && self.is_wall_n(o) || self.is_wall_w(o)
    }

    /// The first square of the collapse area and its steps.
    fn bf_collapse_origin(
        sq: &IsoGridSquare,
        dir: Option<IsoDirections>,
        e: &BentFenceEntry,
        index: i32,
    ) -> (i32, i32, i32, i32) {
        let north = e.is_north();
        let rev_x = if dir == Some(IsoDirections::W) { -1 } else { 1 };
        let rev_y = if dir == Some(IsoDirections::N) { -1 } else { 1 };
        let off = e.collapsed_offset;
        let ox = sq.x
            + if north {
                off - index
            } else if dir == Some(IsoDirections::W) {
                -1
            } else {
                0
            };
        let oy = sq.y
            + if north {
                if dir == Some(IsoDirections::N) {
                    -1
                } else {
                    0
                }
            } else {
                off - index
            };
        (ox, oy, rev_x, rev_y)
    }

    /// `checkCanCollapse(obj, dir, entry)`.
    fn bf_check_can_collapse(
        &self,
        f: Found<'_>,
        dir: Option<IsoDirections>,
        e: &BentFenceEntry,
    ) -> bool {
        if e.collapsed.is_empty() {
            return false;
        }
        let stage = self.bf_bend_stage(f.1, Some(e));
        let Some(tiles) = e.stage(stage) else {
            return false;
        };
        let index = self.bf_tile_index(tiles, f, e, false);
        let (ox, oy, rx, ry) = Self::bf_collapse_origin(f.0, dir, e, index);
        for x in 0..e.collapsed_size_x {
            for y in 0..e.collapsed_size_y {
                let Some(s) = self.cell.get_grid_square(ox + x * rx, oy + y * ry, f.0.z) else {
                    return false;
                };
                if !s.treat_as_solid_floor(self.sprites) {
                    return false;
                }
                for o in s.objects.iter().skip(1).rev() {
                    if self.is_stairs_object(o) {
                        return false;
                    }
                    if self.is_wall_obj(o) && self.wall_blocking_object_on_tile(dir, e, x, y, o) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `isWallBlockingObjectOnTile(dir, entry, x, y, object)`.
    fn wall_blocking_object_on_tile(
        &self,
        dir: Option<IsoDirections>,
        e: &BentFenceEntry,
        x: i32,
        y: i32,
        o: &IsoObject,
    ) -> bool {
        match dir {
            Some(IsoDirections::E) => x != 0,
            Some(IsoDirections::S) => y != 0,
            Some(IsoDirections::W) if x + 1 == e.collapsed_size_x => !self.is_wall_w(o),
            Some(IsoDirections::N) if y + 1 == e.collapsed_size_y => !self.is_wall_n(o),
            _ => true,
        }
    }

    /// `getEntryForObject(obj, dir)`.
    fn bf_entry_for_object(
        &self,
        bf: &BentFences,
        f: Found<'_>,
        dir: Option<IsoDirections>,
    ) -> Option<usize> {
        let name = self.obj_sprite_name(f.1)?;
        let mut matched: Option<usize> = None;
        for &i in bf.fence_map.get(name)?.iter() {
            let e = &bf.entries[i];
            if dir.is_some() && e.dir != dir {
                continue;
            }
            let longer = matched.is_none_or(|m| bf.entries[m].length < e.length);
            if dir.is_none() {
                if !self.bf_valid_object(f, e) || !longer {
                    continue;
                }
                matched = Some(i);
                continue;
            }
            if !self.bf_valid_object(f, e)
                || self.bf_bend_stage(f.1, Some(e)) >= e.stage_count()
                    && !e.collapsed.is_empty()
                    && !self.bf_check_can_collapse(f, dir, e)
                || !longer
            {
                continue;
            }
            matched = Some(i);
        }
        matched
    }

    fn bf(&self) -> Option<std::rc::Rc<BentFences>> {
        self.engine.bent_fences()
    }

    /// `isBendableFence(obj)`: bent or unbent.
    pub fn is_bendable_fence(&self, r: ObjectRef) -> bool {
        let (Some(bf), Some(f)) = (self.bf(), self.find(r)) else {
            return false;
        };
        let e = self
            .bf_entry_for_object(&bf, f, None)
            .map(|i| &bf.entries[i]);
        let stage = self.bf_bend_stage(f.1, e);
        stage > 0 || e.is_some_and(|e| stage < e.stage_count() - 1)
    }

    /// `isUnbentObject(obj, facing)`.
    pub fn is_unbent_object(&self, bf: &BentFences, f: Found<'_>, facing: EdgeFacing) -> bool {
        let Some(e) = self
            .bf_entry_for_object(bf, f, None)
            .map(|i| &bf.entries[i])
        else {
            return false;
        };
        e.facing() == facing && self.bf_bend_stage(f.1, Some(e)) < e.stage_count() - 1
    }

    /// `getThumpData(obj)`.
    pub fn bent_thump_data(&self, r: ObjectRef) -> ThumpData {
        let (Some(bf), Some(f)) = (self.bf(), self.find(r)) else {
            return ThumpData::default();
        };
        let Some(e) = self
            .bf_entry_for_object(&bf, f, None)
            .map(|i| &bf.entries[i])
        else {
            return ThumpData::default();
        };
        let mut d = ThumpData {
            health: e.health,
            ..Default::default()
        };
        let stage = self.bf_bend_stage(f.1, Some(e));
        d.bend_stage = stage;
        d.stages = e.stage_count();
        if !bf.is_enabled() {
            d.thumpers_to_damage = -1;
            d.damage_multiplier = bf.damage_multiplier;
            return d;
        }
        let mut to_damage = bf.thumpers_required;
        if stage > 0 {
            d.direction_bent = e.dir;
            to_damage = (bf.thumpers_required / (stage + 1)).max(1);
        }
        d.thumpers_to_damage = to_damage;
        d.damage_multiplier = bf.damage_multiplier;
        let Some(tiles) = e.stage(stage) else {
            return d;
        };
        let index = self.bf_tile_index(tiles, f, e, false);
        let count = |x: i32, y: i32, z: i32| -> i32 {
            if self.cell.get_grid_square(x, y, z).is_none() {
                return 0;
            }
            self.engine.zombies_on_square((x, y, z)).len() as i32
        };
        let exists = |x: i32, y: i32, z: i32| self.cell.get_grid_square(x, y, z).is_some();
        let (mut n, mut s, mut ea, mut w) = (0, 0, 0, 0);
        let z = f.0.z;
        for i in 0..tiles.len() as i32 {
            let Some(sq) = self.bf_span_square(f.0, e, i, index) else {
                continue;
            };
            let (x, y) = (sq.x, sq.y);
            let here = count(x, y, z);
            if e.is_north() {
                d.total_thumpers += here;
                s += here;
                if exists(x, y - 1, z) {
                    let c = count(x, y - 1, z);
                    n += c;
                    d.total_thumpers += c;
                    if exists(x, y - 2, z) {
                        let c = count(x, y - 2, z);
                        n += c;
                        d.total_thumpers += c;
                    }
                }
                if exists(x, y + 1, z) {
                    let c = count(x, y + 1, z);
                    s += c;
                    d.total_thumpers += c;
                }
                d.direction_to_bend = Some(if n > s {
                    IsoDirections::S
                } else {
                    IsoDirections::N
                });
                continue;
            }
            d.total_thumpers += here;
            ea += here;
            if exists(x - 1, y, z) {
                let c = count(x - 1, y, z);
                w += c;
                d.total_thumpers += c;
                if exists(x - 2, y, z) {
                    let c = count(x - 2, y, z);
                    w += c;
                    d.total_thumpers += c;
                }
            }
            if exists(x + 1, y, z) {
                let c = count(x + 1, y, z);
                ea += c;
                d.total_thumpers += c;
            }
            d.direction_to_bend = Some(if ea > w {
                IsoDirections::W
            } else {
                IsoDirections::E
            });
        }
        d
    }

    /// `checkDamageHoppableFence(thumper, sq, oppositeSq)` for a zombie.
    pub fn check_damage_hoppable_fence(&mut self, from: (i32, i32, i32), opp: (i32, i32, i32)) {
        let Some(bf) = self.bf() else { return };
        if !bf.is_enabled() {
            return;
        }
        let Some(r) = self.hoppable_between_squares(from, opp) else {
            return;
        };
        if !self.is_bendable_fence(r) {
            return;
        }
        let d = self.bent_thump_data(r);
        let roll = self.engine.rand().next_range(0, 1) as f32;
        let damage = self.object_damage(r);
        let v =
            (damage as f32 - roll * d.damage_multiplier * d.damage_modifier) as i32 as i16 as i32;
        self.runtime.object_damage.insert(r, v);
        if v <= 0 {
            self.bend_fence(r, d.direction_bent);
            if let Some(r) = self.find_ref(r) {
                self.runtime.object_damage.insert(r, d.health);
            }
        }
    }

    /// The object `r` still names, if it is in the world.
    fn find_ref(&self, r: ObjectRef) -> Option<ObjectRef> {
        self.find(r).map(|_| r)
    }

    /// `IsoSpriteManager.instance.getSprite(name)` (added when unknown).
    fn sprite_named(&mut self, name: &str) -> usize {
        match self.sprites.named_map.get(name) {
            Some(&s) => s,
            None => self.sprites.add_sprite_named(name),
        }
    }

    /// `obj.setSprite(getSprite(name))`.
    fn set_object_sprite(&mut self, r: ObjectRef, name: &str) {
        let s = self.sprite_named(name);
        if let Some(o) = self.find_mut(r) {
            o.sprite = Some(s);
        }
    }

    /// `doSquareUpdateTasks(square)`.
    fn square_update_tasks(&mut self, p: (i32, i32, i32)) {
        self.cell
            .recalc_all_with_neighbours(p.0, p.1, p.2, self.meta, self.sprites);
        self.engine.square_changed(p);
    }

    /// `square.AddTileObject(IsoObject.getNew(square, name), index)`.
    fn add_tile_object(&mut self, p: (i32, i32, i32), name: &str, index: i32) {
        let s = self.sprite_named(name);
        let sprites = &*self.sprites;
        let Some(sq) = self.cell.get_grid_square_mut(p.0, p.1, p.2) else {
            return;
        };
        let id = sq.new_object_id();
        let obj = IsoObject::new(id, ObjectKind::Object, Some(s));
        let i = sq.place_wall_and_door_check(sprites, &obj, index);
        if i >= 0 && (i as usize) <= sq.objects.len() {
            sq.objects.insert(i as usize, obj);
        } else {
            sq.objects.push(obj);
        }
        sq.burnt_out = false;
        self.cell
            .recalc_all_with_neighbours(p.0, p.1, p.2, self.meta, self.sprites);
        self.engine.square_changed(p);
    }

    /// `bendFence(obj, dir)` (`dir` is null for an unbent fence's
    /// `directionBent`; the original then throws in its switch when the
    /// object is no valid fence, which here only stops the bend).
    pub fn bend_fence(&mut self, r: ObjectRef, dir: Option<IsoDirections>) {
        let Some(bf) = self.bf() else { return };
        let Some(f) = self.find(r) else { return };
        let mut dir = dir;
        let mut e = self.bf_entry_for_object(&bf, f, dir);
        if !e.is_some_and(|i| self.bf_valid_object(f, &bf.entries[i])) {
            let d2 = match dir {
                None => return,
                Some(IsoDirections::N) => Some(IsoDirections::W),
                Some(IsoDirections::S) => Some(IsoDirections::E),
                Some(IsoDirections::W) => Some(IsoDirections::N),
                Some(IsoDirections::E) => Some(IsoDirections::S),
                other => other,
            };
            if d2 != dir {
                e = self.bf_entry_for_object(&bf, f, d2);
                dir = d2;
            }
        }
        let Some(ei) = e else { return };
        let entry = &bf.entries[ei];
        let old = self.bf_bend_stage(f.1, Some(entry));
        let thump_sound = self
            .env()
            .obj_prop(f.1, "ThumpSound")
            .unwrap_or("ZombieThumpGeneric")
            .to_string();
        let sqp = (f.0.x, f.0.y, f.0.z);
        self.swap_tiles(r, dir, true, -1);
        let new = self.find(r).map_or(-1, |f| {
            let e2 = self
                .bf_entry_for_object(&bf, f, None)
                .map(|i| &bf.entries[i]);
            self.bf_bend_stage(f.1, e2)
        });
        if new <= old {
            return;
        }
        let n = entry.stage_count();
        let sound = match thump_sound.as_str() {
            "ZombieThumpChainlinkFence" => {
                if new == 1 {
                    "ZombieThumpChainlinkFenceDamageLow"
                } else if new < n - 1 {
                    "ZombieThumpChainlinkFenceDamageHigh"
                } else {
                    "ZombieThumpChainlinkFenceDamageCollapse"
                }
            }
            "ZombieThumpMetalPoleFence" => {
                if new == 1 {
                    "ZombieThumpMetalPoleFenceDamageLow"
                } else if new < n - 1 {
                    "ZombieThumpMetalPoleFenceDamageHigh"
                } else {
                    "ZombieThumpMetalPoleFenceDamageCollapse"
                }
            }
            "ZombieThumpWood" if new >= n => "ZombieThumpWoodCollapse",
            _ => "BreakObject",
        };
        self.engine.play_world_sound(sound, sqp);
    }

    /// `swapTiles(obj, dir, bending, forceStage)`.
    pub fn swap_tiles(
        &mut self,
        r: ObjectRef,
        dir: Option<IsoDirections>,
        bending: bool,
        force_stage: i32,
    ) {
        let Some(bf) = self.bf() else { return };
        let Some(f) = self.find(r) else { return };
        let ei = match self.bf_entry_for_object(&bf, f, dir) {
            Some(i) => i,
            None if bending => match self.bf_entry_for_object(&bf, f, None) {
                Some(i) => i,
                None => return,
            },
            None => return,
        };
        let e = &bf.entries[ei];
        if let Some(d) = dir {
            if e.is_north() && d != IsoDirections::N && d != IsoDirections::S {
                return;
            }
            if !e.is_north() && d != IsoDirections::W && d != IsoDirections::E {
                return;
            }
        }
        let stage = if force_stage != -1 {
            force_stage
        } else {
            self.bf_bend_stage(f.1, Some(e))
        };
        if stage == -1 || !bending && stage == 0 {
            return;
        }
        let n = e.stage_count();
        let do_smash = bending && stage > 0 && e.dir != dir;
        let do_collapse = !do_smash && bending && stage + 1 >= n - 1;
        let do_smash_collapse = do_collapse && !self.bf_check_can_collapse(f, dir, e) && e.do_smash;
        let do_remove_collapsed = !bending && stage + 1 == n;
        let do_debris = do_collapse && !e.debris.is_empty();
        if stage == n && bending {
            return;
        }
        let (Some(tiles), Some(base), Some(swap)) = (
            e.stage(stage),
            e.stage(0),
            e.stage(stage + if bending { 1 } else { -1 }),
        ) else {
            return;
        };
        if tiles.len() != swap.len() {
            return;
        }
        let index = self.bf_tile_index(tiles, f, e, false);
        let origin = f.0;
        let (ox, oy, oz) = (origin.x, origin.y, origin.z);
        let mut updated = Vec::new();
        for i in 0..tiles.len() as i32 {
            let x = ox + if e.is_north() { i - index } else { 0 };
            let y = oy + if e.is_north() { 0 } else { i - index };
            let Some(sq) = self.cell.get_grid_square(x, y, oz) else {
                continue;
            };
            let Some(fo) = self.bf_object_for_entry(sq, tiles, i as usize) else {
                continue;
            };
            let fr = ObjectRef {
                x,
                y,
                z: oz,
                id: fo.id,
            };
            if self.env().obj_has_prop(fo, "CornerNorthWall") {
                self.split_corner(fr, e);
            }
            if bending {
                let signs: Vec<ObjectRef> = self
                    .cell
                    .get_grid_square(x, y, oz)
                    .map(|sq| {
                        sq.objects
                            .iter()
                            .skip(1)
                            .rev()
                            .filter(|o| {
                                o.sprite.is_some()
                                    && ["attachedN", "attachedW", "attachedS", "attachedE"]
                                        .iter()
                                        .any(|a| self.obj_flag(o, a))
                            })
                            .map(|o| ObjectRef {
                                x,
                                y,
                                z: oz,
                                id: o.id,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for s in signs {
                    if let Some(o) = self.remove_object(s, false) {
                        self.engine.broken_fence_items((x, y, oz), &o);
                    }
                }
            }
            let name = if do_smash || do_smash_collapse {
                &base[i as usize]
            } else {
                &swap[i as usize]
            };
            self.set_object_sprite(fr, name);
            if do_debris {
                self.add_debris_object(fr, dir, e);
            }
            updated.push((x, y, oz));
            if self.find(fr).is_some() {
                self.runtime
                    .object_damage
                    .insert(fr, e.health as i16 as i32);
            }
        }
        for p in updated {
            self.square_update_tasks(p);
        }
        if let Some(d) = dir {
            if do_smash || do_smash_collapse {
                self.smash_fence(r, d, index);
            } else if do_collapse {
                self.collapse(r, d, e, index);
            }
        } else if do_remove_collapsed {
            self.remove_collapsed_tiles(r, e.dir, e, index);
        }
    }

    /// `splitCorner(obj, square, entry)`.
    fn split_corner(&mut self, r: ObjectRef, e: &BentFenceEntry) {
        let Some((_, o)) = self.find(r) else { return };
        let env = self.env();
        let (Some(cn), Some(cw)) = (
            env.obj_prop(o, "CornerNorthWall").map(str::to_string),
            env.obj_prop(o, "CornerWestWall").map(str::to_string),
        ) else {
            return;
        };
        let (keep, add) = if e.is_north() { (cn, cw) } else { (cw, cn) };
        self.set_object_sprite(r, &keep);
        let p = (r.x, r.y, r.z);
        let len = self
            .cell
            .get_grid_square(p.0, p.1, p.2)
            .map_or(0, |s| s.objects.len() as i32);
        self.add_tile_object(p, &add, len + 1);
        self.square_update_tasks(p);
    }

    /// `addDebrisObject(obj, dir, entry)`.
    fn add_debris_object(&mut self, r: ObjectRef, dir: Option<IsoDirections>, e: &BentFenceEntry) {
        let Some((_, o)) = self.find(r) else { return };
        if self.is_wall_obj(o) || e.debris.is_empty() {
            return;
        }
        let o = o.clone();
        // PZArrayUtil.pickRandom: Rand.Next(size).
        let name = e.debris[self.engine.rand().next(e.debris.len() as i32) as usize].clone();
        let mut p = (r.x, r.y, r.z);
        if matches!(dir, Some(IsoDirections::N | IsoDirections::W)) {
            let (dx, dy) = dir.map_or((0, 0), |d| d.delta());
            p = (p.0 + dx, p.1 + dy, p.2);
        }
        let Some(sq) = self.cell.get_grid_square(p.0, p.1, p.2) else {
            return;
        };
        if !sq.treat_as_solid_floor(self.sprites) {
            return;
        }
        let index = if p == (r.x, r.y, r.z) {
            sq.object_index(r.id).map_or(-1, |i| i as i32)
        } else {
            -1
        };
        self.add_tile_object(p, &name, index);
        self.engine.broken_fence_items(p, &o);
    }

    /// `smashFence(obj, dir, index)`.
    pub fn smash_fence(&mut self, r: ObjectRef, dir: IsoDirections, index: i32) {
        let Some(bf) = self.bf() else { return };
        let Some(f) = self.find(r) else { return };
        let Some(ei) = self.bf_entry_for_object(&bf, f, None) else {
            return;
        };
        let e = &bf.entries[ei];
        let Some(tiles) = e.stage(0) else { return };
        let stage = self.bf_bend_stage(f.1, Some(e));
        let use_index = if index != -1 {
            index
        } else {
            self.bf_tile_index(tiles, f, e, true)
        };
        let (ox, oy, oz) = (f.0.x, f.0.y, f.0.z);
        if stage != 0 {
            self.reset_fence(r);
        }
        for i in 1..tiles.len().saturating_sub(1) as i32 {
            let x = ox + if e.is_north() { i - use_index } else { 0 };
            let y = oy + if e.is_north() { 0 } else { i - use_index };
            let Some(sq) = self.cell.get_grid_square(x, y, oz) else {
                continue;
            };
            let fo = self
                .bf_object_for_entry(sq, tiles, i as usize)
                .map(|o| o.id)
                .or_else(|| self.breakable_object(sq, e.is_north()));
            let Some(id) = fo else { continue };
            let fr = ObjectRef { x, y, z: oz, id };
            let name = tiles[i as usize].clone();
            self.set_object_sprite(fr, &name);
            if self.engine.is_breakable_fence(&name) {
                self.engine.destroy_fence(self.cell, fr, dir);
                continue;
            }
            if let Some((_, o)) = self.find(fr) {
                let o = o.clone();
                self.engine.broken_fence_items((x, y, oz), &o);
            }
            self.remove_object(fr, false);
            self.square_update_tasks((x, y, oz));
        }
    }

    /// `BrokenFences.getBreakableObject(square, north)`.
    fn breakable_object(
        &self,
        sq: &IsoGridSquare,
        north: bool,
    ) -> Option<pz_world::objects::ObjectId> {
        sq.objects
            .iter()
            .find(|o| {
                self.obj_sprite_name(o)
                    .is_some_and(|n| self.engine.is_breakable_fence(n))
                    && (north && self.obj_flag(o, "collideN")
                        || !north && self.obj_flag(o, "collideW"))
            })
            .map(|o| o.id)
    }

    /// `resetFence(obj)`.
    pub fn reset_fence(&mut self, r: ObjectRef) {
        let Some(bf) = self.bf() else { return };
        let Some(f) = self.find(r) else { return };
        let Some(ei) = self.bf_entry_for_object(&bf, f, None) else {
            return;
        };
        let e = &bf.entries[ei];
        let stage = self.bf_bend_stage(f.1, Some(e));
        if stage == -1 {
            return;
        }
        let (Some(tiles), Some(base)) = (e.stage(stage), e.stage(0)) else {
            return;
        };
        if tiles.len() != base.len() {
            return;
        }
        let index = self.bf_tile_index(tiles, f, e, false);
        let (ox, oy, oz) = (f.0.x, f.0.y, f.0.z);
        for i in 0..tiles.len() as i32 {
            let x = ox + if e.is_north() { i - index } else { 0 };
            let y = oy + if e.is_north() { 0 } else { i - index };
            let Some(sq) = self.cell.get_grid_square(x, y, oz) else {
                continue;
            };
            let Some(fo) = self.bf_object_for_entry(sq, tiles, i as usize) else {
                continue;
            };
            let fr = ObjectRef {
                x,
                y,
                z: oz,
                id: fo.id,
            };
            if self.env().obj_has_prop(fo, "CornerNorthWall") {
                self.split_corner(fr, e);
            }
            self.set_object_sprite(fr, &base[i as usize]);
            self.square_update_tasks((x, y, oz));
            self.runtime
                .object_damage
                .insert(fr, e.health as i16 as i32);
        }
        if stage + 1 == e.stage_count() {
            self.remove_collapsed_tiles(r, e.dir, e, index);
        }
    }

    /// `removeCollapsedTiles(obj, dir, entry, index)`.
    fn remove_collapsed_tiles(
        &mut self,
        r: ObjectRef,
        dir: Option<IsoDirections>,
        e: &BentFenceEntry,
        index: i32,
    ) {
        let Some(bf) = self.bf() else { return };
        let Some((sq, _)) = self.find(r) else { return };
        let (ox, oy, rx, ry) = Self::bf_collapse_origin(sq, dir, e, index);
        let z = sq.z;
        for x in 0..e.collapsed_size_x {
            for y in 0..e.collapsed_size_y {
                let p = (ox + x * rx, oy + y * ry, z);
                let Some(s) = self.cell.get_grid_square(p.0, p.1, p.2) else {
                    continue;
                };
                let n = s.objects.len();
                for i in (1..n).rev() {
                    let Some(s) = self.cell.get_grid_square(p.0, p.1, p.2) else {
                        break;
                    };
                    let Some(o) = s.objects.get(i) else { continue };
                    if self
                        .obj_sprite_name(o)
                        .is_some_and(|nm| bf.collapsed_tiles.contains(nm))
                    {
                        let id = o.id;
                        self.remove_object(
                            ObjectRef {
                                x: p.0,
                                y: p.1,
                                z: p.2,
                                id,
                            },
                            false,
                        );
                    }
                    self.square_update_tasks(p);
                }
            }
        }
    }

    /// `collapse(obj, dir, entry, index)`.
    fn collapse(&mut self, r: ObjectRef, dir: IsoDirections, e: &BentFenceEntry, index: i32) {
        let Some(bf) = self.bf() else { return };
        let Some((sq, _)) = self.find(r) else { return };
        let north = e.is_north();
        let (ox, oy, rx, ry) = Self::bf_collapse_origin(sq, Some(dir), e, index);
        let z = sq.z;
        let mut collapsed = e.collapsed.iter();
        let mut trip = Vec::new();
        let mut crush = Vec::new();
        for x in 0..e.collapsed_size_x {
            for y in 0..e.collapsed_size_y {
                let p = (ox + x * rx, oy + y * ry, z);
                if self.cell.get_grid_square(p.0, p.1, p.2).is_none() {
                    continue;
                }
                if north && y == 0 || !north && x == 0 {
                    let (xo, yo) = match dir {
                        IsoDirections::N => (0, 1),
                        IsoDirections::S => (0, -1),
                        IsoDirections::E => (-1, 0),
                        IsoDirections::W => (1, 0),
                        _ => (0, 0),
                    };
                    // The original offsets the collapse origin, not this
                    // square.
                    if self.cell.get_grid_square(ox + xo, oy + yo, z).is_some() {
                        trip.extend(self.engine.moving_objects_on_square((ox + xo, oy + yo, z)));
                    }
                }
                crush.extend(self.engine.moving_objects_on_square(p));
                let n = self
                    .cell
                    .get_grid_square(p.0, p.1, p.2)
                    .map_or(0, |s| s.objects.len());
                for i in (1..n).rev() {
                    let Some(s) = self.cell.get_grid_square(p.0, p.1, p.2) else {
                        break;
                    };
                    let Some(o) = s.objects.get(i) else { continue };
                    let or = ObjectRef {
                        x: p.0,
                        y: p.1,
                        z: p.2,
                        id: o.id,
                    };
                    let name = self.obj_sprite_name(o).map(str::to_string);
                    if name
                        .as_deref()
                        .is_some_and(|n| self.engine.is_breakable_fence(n))
                    {
                        self.engine.destroy_fence(self.cell, or, dir);
                        continue;
                    }
                    let Some(name) = name else { continue };
                    if self.is_wall_obj(o)
                        || self.env().obj_has_flag(o, "solidfloor")
                        || bf.fence_map.contains_key(&name)
                        || bf.collapsed_tiles.contains(&name)
                    {
                        continue;
                    }
                    let o = o.clone();
                    if o.container.as_ref().is_some_and(|c| !c.items.is_empty()) {
                        self.engine.dump_contents(p, &o);
                    }
                    self.remove_object(or, false);
                    self.engine.broken_fence_items(p, &o);
                }
                let Some(name) = collapsed.next() else {
                    continue;
                };
                let len = self
                    .cell
                    .get_grid_square(p.0, p.1, p.2)
                    .map_or(0, |s| s.objects.len() as i32);
                self.add_tile_object(p, name, len + 1);
                self.square_update_tasks(p);
            }
        }
        self.engine.fence_collapsed(r, &trip, &crush);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(
        dir: IsoDirections,
        stages: &[&[&str]],
        collapsed: Option<(i32, i32, &[&str])>,
    ) -> FenceTileDef {
        let v = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        FenceTileDef {
            dir: Some(dir),
            health: -1,
            collapsed_offset: 1,
            collapsed_size_x: collapsed.map_or(-1, |c| c.0),
            collapsed_size_y: collapsed.map_or(-1, |c| c.1),
            do_smash: false,
            stages: Some(
                stages
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (i as i32, v(t)))
                    .collect(),
            ),
            collapsed: collapsed.map(|c| v(c.2)),
            debris: None,
        }
    }

    #[test]
    fn add_fence_tiles_keeps_lua_order_and_drops_bad_collapses() {
        let mut bf = BentFences::new(50, 1.0);
        assert!(bf.is_enabled());
        bf.add_fence_tiles(
            2,
            &[
                def(
                    IsoDirections::N,
                    &[&["a_0", "a_1", "a_0"], &["b_0", "b_1", "b_2"]],
                    Some((1, 2, &["c_0", "c_1"])),
                ),
                // 2 x 2 collapse with three tiles: the entry is dropped.
                def(
                    IsoDirections::S,
                    &[&["a_0", "a_1", "a_0"], &["d_0", "d_1", "d_2"]],
                    Some((2, 2, &["e_0", "e_1", "e_2"])),
                ),
                FenceTileDef {
                    stages: None,
                    ..Default::default()
                },
            ],
        );
        assert_eq!(bf.entries.len(), 1);
        let e = &bf.entries[0];
        assert_eq!(
            (e.health, e.length, e.facing()),
            (100, 3, EdgeFacing::of_north(true))
        );
        // A tile listed twice in a stage maps to the entry twice.
        assert_eq!(bf.fence_map["a_0"], vec![0, 0]);
        assert_eq!(bf.fence_map["b_2"], vec![0]);
        assert!(bf.collapsed_tiles.contains("c_1") && !bf.collapsed_tiles.contains("e_0"));
        assert!(!BentFences::new(-1, 1.0).is_enabled());
    }
}
