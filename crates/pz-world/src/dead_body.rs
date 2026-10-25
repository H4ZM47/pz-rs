//! `IsoDeadBody` in the world: joining and leaving it (`addToWorld`,
//! `removeFromWorld`, `removeFromSquare`), the `ObjectIDType.DeadBody`
//! registry `updateBodies` walks, corpse rot (`updateBodies`,
//! `updateRotting`, `updateAnimalRotting`, the fake dead waking up), the
//! static updaters' reanimation (`update`, `reanimate`), corpse counts
//! ([`corpse_count`]) and the flies ([`flies_sound`]).
//!
//! Single player only (`GameClient.client` / `GameServer.server` false):
//! object-change packets are not ported.
//!
//! The engine runs, once per `IsoCell.update` after the static updaters
//! ([`process_static_updaters`]), [`update_bodies`]; and around chunk
//! loading and unloading [`chunk_loaded`] / [`add_loaded_corpses_to_world`]
//! and [`chunk_removed_from_world`].

pub mod corpse_count;
pub mod flies_sound;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use pz_inventory::item::ItemKind;
use pz_inventory::shared::ItemRef;
use pz_inventory::InventoryItem;

use crate::chunk::IsoCell;
use crate::java::{fastfloor, JavaLongHashMap};
use crate::save::lua_table::{LuaKey, LuaTable, LuaValue};
use crate::save::visual::{CorpseVisual, IsoDeadBody};
use crate::square_mutation::{render_square, LuaEvent, MutationEnv, SquarePos};

/// `IsoDeadBody.MAX_ROT_STAGES`.
pub const MAX_ROT_STAGES: i32 = 3;
/// `IsoDeadBody.MAX_ROT_STAGES_ANIMALS`.
pub const MAX_ROT_STAGES_ANIMALS: i32 = 4;
/// The item rotting corpses spawn.
pub const MAGGOTS: &str = "Maggots";
/// `ObjectIDType.DeadBody.index`.
pub const DEAD_BODY_ID_TYPE: i8 = 4;

/// `IsoObject.invalidateRenderChunkLevel` flag rot changes set
/// (`FBORenderChunk.DIRTY_CORPSE`).
const DIRTY_CORPSE: u64 = 2;
/// `reanimate()`'s `invalidateRenderChunkLevel(128)`.
const DIRTY_OBJECT_REMOVE: u64 = 128;

/// `ObjectIDType.DeadBody` of `ObjectIDManager` (the corpses `updateBodies`
/// walks) and the corpses in `IsoCell.staticUpdaterObjectList`.
#[derive(Clone, Debug, Default)]
pub struct DeadBodies {
    /// `idToObjectMap.getHashMap()`'s key order.
    order: JavaLongHashMap,
    /// Where each registered corpse lies.
    at: HashMap<i64, SquarePos>,
    /// `lastId` (`ObjectIDManager.load`: the saved value plus 100).
    pub last_id: i64,
    /// `countNewId`.
    pub count_new_id: i64,
    /// `staticUpdaterObjectList` corpses, by id, in list order.
    static_updaters: Vec<i64>,
}

impl DeadBodies {
    /// `ObjectIDType.allocateID()`.
    fn allocate_id(&mut self) -> i64 {
        self.last_id += 1;
        self.count_new_id += 1;
        self.last_id
    }

    /// `ObjectIDManager.addObject(body)`: a body without an id (-1) gets
    /// the next free one (as a `short`).
    pub fn add_object(&mut self, pos: SquarePos, body: &mut IsoDeadBody) {
        let mut id = body.object_id.0 as i64;
        if id == -1 {
            id = self.allocate_id() as i16 as i64;
            while self.order.contains(id) {
                id = self.allocate_id() as i16 as i64;
            }
        }
        self.order.insert(id);
        self.at.insert(id, pos);
        body.object_id = (id as i16, DEAD_BODY_ID_TYPE);
    }

    /// `ObjectIDManager.remove(id)`.
    pub fn remove_object(&mut self, id: i64) {
        if self.order.remove(id) {
            self.at.remove(&id);
        }
    }

    /// The registered ids in `getObjects()` order.
    pub fn ids(&self) -> Vec<i64> {
        self.order.order()
    }

    /// The square of registered corpse `id`.
    pub fn position(&self, id: i64) -> Option<SquarePos> {
        self.at.get(&id).copied()
    }

    /// `staticUpdaterObjectList`'s corpses.
    pub fn static_updaters(&self) -> &[i64] {
        &self.static_updaters
    }

    /// `addToStaticUpdaterObjectList(body)`.
    pub fn add_static_updater(&mut self, id: i64) {
        if !self.static_updaters.contains(&id) {
            self.static_updaters.push(id);
        }
    }

    /// `removeFromStaticUpdaterObjectList(body)`.
    pub fn remove_static_updater(&mut self, id: i64) {
        if let Some(i) = self.static_updaters.iter().position(|&e| e == id) {
            self.static_updaters.remove(i);
        }
    }

    /// The body registered as `id`: the last one on its square with that id.
    pub fn find<'c>(
        &self,
        cell: &'c IsoCell,
        id: i64,
    ) -> Option<(SquarePos, usize, &'c IsoDeadBody)> {
        let pos = self.position(id)?;
        let sq = cell.get_grid_square(pos.0, pos.1, pos.2)?;
        let i = sq
            .static_moving
            .iter()
            .rposition(|b| b.object_id.0 as i64 == id)?;
        Some((pos, i, &sq.static_moving[i]))
    }

    fn index_of(&self, cell: &IsoCell, id: i64) -> Option<(SquarePos, usize)> {
        self.find(cell, id).map(|(p, i, _)| (p, i))
    }
}

/// The game state corpse rot reads.
#[derive(Clone, Debug, PartialEq)]
pub struct CorpseSettings {
    /// `SandboxOptions.hoursForCorpseRemoval`.
    pub hours_for_corpse_removal: f64,
    /// `ServerOptions.removePlayerCorpsesOnCorpseRemoval`.
    pub remove_player_corpses_on_corpse_removal: bool,
    /// `SandboxOptions.lore.disableFakeDead` (2: rotting zombies may turn
    /// fake dead; 3: none are).
    pub disable_fake_dead: i32,
    /// `SandboxOptions.maggotSpawn` (2: not on the ground; 3: none).
    pub maggot_spawn: i32,
    /// `SandboxOptions.lore.reanimate`.
    pub reanimate: i32,
    /// `Core.tutorial`.
    pub tutorial: bool,
    /// `ClimateManager.getSeasonName()`.
    pub season: String,
}

impl Default for CorpseSettings {
    fn default() -> Self {
        CorpseSettings {
            hours_for_corpse_removal: -1.0,
            remove_player_corpses_on_corpse_removal: false,
            disable_fake_dead: 1,
            maggot_spawn: 1,
            reanimate: 3,
            tutorial: false,
            season: String::new(),
        }
    }
}

/// What the fake dead's player check reads about a local player
/// (`IsoPlayer.players[pn]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorpsePlayer {
    pub x: f32,
    pub y: f32,
    pub dead: bool,
    /// `isGhostMode() || isInvisible()`.
    pub ghost_or_invisible: bool,
    /// `getVehicle() != null`.
    pub in_vehicle: bool,
}

/// What corpse rot and reanimation need from other systems.
pub trait CorpseHooks {
    /// `InventoryItemFactory.CreateItem(fullType)` (`Maggots`), drawing
    /// from the same `Rand` as the world.
    fn create_item(
        &mut self,
        full_type: &str,
        rand: &mut dyn crate::GameRand,
    ) -> Option<InventoryItem>;
    /// `IsoCell.addToProcessItems(item)` of an item put in a corpse.
    fn add_to_process_items(&mut self, _item: &InventoryItem) {}
    /// `IsoPlayer.players[pn]`.
    fn player(&self, _pn: i32) -> Option<CorpsePlayer> {
        None
    }
    /// `square.isCanSee(pn)`.
    fn can_see(&self, _pos: SquarePos, _pn: i32) -> bool {
        false
    }
    /// The corpse's rot stage or skeleton changed (`atlasTex = null`,
    /// `invalidateCorpse()`): its `DeadBodyAtlas` entry is stale.
    fn corpse_visual_changed(&mut self, _pos: SquarePos, _body: &IsoDeadBody) {}
    /// The first part of `reanimate()`: the zombie (or `reanimateAnimal`'s
    /// animal) made from the body, joining the cell's lists, with its
    /// `Rand` draws (toughness). Returns false to keep the body (the
    /// server's `getUniqueZombieId` failing).
    fn reanimate(
        &mut self,
        _pos: SquarePos,
        _body: &IsoDeadBody,
        _rand: &mut dyn crate::GameRand,
    ) -> bool {
        false
    }
    /// The rest of `reanimate()` after the body left the world and its
    /// square (and, for a human, `OnContainerUpdate` fired): the zombie's
    /// reanimate timer, worn items, action context and animator update.
    fn reanimated(&mut self, _pos: SquarePos, _body: IsoDeadBody) {}
    /// `LuaEventManager.triggerEvent("OnContainerUpdate")` (no argument).
    fn container_update_event(&mut self) {}
}

/// How [`update_bodies`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateBodiesOutcome {
    Done,
    /// A corpse without a container became a skeleton: the original's
    /// `getContainer().clear()` throws a `NullPointerException` there,
    /// ending `updateBodies` (and the rest of `IsoCell.update`).
    NullContainer,
}

/// The body's square from its position (`getXi()`, `getYi()`, `getZi()`).
fn body_xyz(b: &IsoDeadBody) -> (i32, i32, i32) {
    (
        fastfloor(b.base.x),
        fastfloor(b.base.y),
        fastfloor(b.base.z),
    )
}

/// `IsoObject.invalidateRenderChunkLevel(flags)` of a corpse on `pos`: its
/// square's chunk level and its render square's.
fn invalidate_body(env: &mut MutationEnv<'_>, pos: SquarePos, flags: u64) {
    let mut squares = vec![pos];
    if let Some(r) = render_square(env.cell, pos) {
        if r != pos {
            squares.push(r);
        }
    }
    for p in squares {
        let (wx, wy) = (p.0.div_euclid(8), p.1.div_euclid(8));
        if env.cell.get_grid_square(p.0, p.1, p.2).is_some() {
            env.render.invalidate_render_chunk_level(wx, wy, p.2, flags);
        }
    }
}

/// `IsoDeadBody.addToWorld()` of `staticMovingObjects[index]` on `pos`:
/// `CorpseCount.corpseAdded`, the hook (`IsoObject.addToWorld`'s
/// registries, `FliesSound.corpseAdded`, `DesignationZoneAnimal`),
/// `ObjectIDManager.addObject`, the static updaters when a reanimation is
/// set, and the death time clamped to the world age
/// ([`crate::square_mutation::SquareMutationHooks::world_age_hours`]).
pub fn add_to_world(env: &mut MutationEnv<'_>, pos: SquarePos, index: usize) {
    let Some(body) = env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)
        .and_then(|s| s.static_moving.get(index))
    else {
        return;
    };
    let (x, y, z) = body_xyz(body);
    corpse_count::corpse_added(env.cell, env.meta, x, y, z);
    let world_age = env.world.world_age_hours();
    let IsoCell {
        chunks,
        dead_bodies,
        ..
    } = &mut *env.cell;
    let (wx, wy) = (pos.0.div_euclid(8), pos.1.div_euclid(8));
    let Some(body) = chunks
        .get_mut(&(wx, wy))
        .and_then(|c| c.get_grid_square_mut(pos.0 - wx * 8, pos.1 - wy * 8, pos.2))
        .and_then(|s| s.static_moving.get_mut(index))
    else {
        return;
    };
    env.world.corpse_added_to_world(pos, body);
    dead_bodies.add_object(pos, body);
    if body.reanimate_time > 0.0 {
        dead_bodies.add_static_updater(body.object_id.0 as i64);
    }
    if let Some(age) = world_age {
        let age = age as f32;
        if body.death_time < 0.0 {
            body.death_time = age;
        }
        if body.death_time > age {
            body.death_time = age;
        }
    }
}

/// `body.removeFromWorld(); body.removeFromSquare();` of
/// `staticMovingObjects[index]` on `pos`: `CorpseCount.corpseRemoved`, the
/// hook (`FliesSound.corpseRemoved`, `DesignationZoneAnimal`, the
/// `IsoObject.removeFromWorld` registries), `ObjectIDManager.remove`, the
/// static updaters, then the body leaves the square.
pub fn remove_from_world_and_square(
    env: &mut MutationEnv<'_>,
    pos: SquarePos,
    index: usize,
) -> Option<IsoDeadBody> {
    let body = env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)?
        .static_moving
        .get(index)?;
    let (x, y, z) = body_xyz(body);
    let id = body.object_id.0 as i64;
    corpse_count::corpse_removed(env.cell, env.meta, x, y, z);
    let mut body = env
        .cell
        .get_grid_square_mut(pos.0, pos.1, pos.2)?
        .static_moving
        .remove(index);
    env.world.corpse_removed_from_world(pos, &mut body);
    env.cell.dead_bodies.remove_object(id);
    env.cell.dead_bodies.remove_static_updater(id);
    Some(body)
}

/// `IsoChunk.doLoadGridsquare`'s corpse part before its corpses join the
/// world: `CorpseCount.chunkLoaded` (the engine then calls
/// [`flies_sound::FliesSound::chunk_loaded`]).
pub fn chunk_loaded(cell: &mut IsoCell, wx: i32, wy: i32) {
    if let Some(ch) = cell.chunks.get_mut(&(wx, wy)) {
        corpse_count::chunk_loaded(ch);
    }
}

/// `doLoadGridsquare`'s loop over the chunk's squares (levels up, then x,
/// then y) calling `addToWorld()` on every `staticMovingObjects` corpse.
pub fn add_loaded_corpses_to_world(env: &mut MutationEnv<'_>, wx: i32, wy: i32) {
    let Some((min, max)) = env
        .cell
        .chunks
        .get(&(wx, wy))
        .map(|c| (c.min_level, c.max_level))
    else {
        return;
    };
    for z in min..=max {
        for x in 0..8 {
            for y in 0..8 {
                let pos = (wx * 8 + x, wy * 8 + y, z);
                let mut i = 0;
                while i < env
                    .cell
                    .get_grid_square(pos.0, pos.1, pos.2)
                    .map_or(0, |s| s.static_moving.len())
                {
                    add_to_world(env, pos, i);
                    i += 1;
                }
            }
        }
    }
}

/// `IsoChunk.removeFromWorld`'s corpse part: each corpse's
/// `removeFromWorld()` (levels up, squares in index order; the bodies stay
/// on their squares), then `corpseCount.removeFromWorld()` (the engine
/// also calls [`flies_sound::FliesSound::chunk_removed_from_world`]).
pub fn chunk_removed_from_world(env: &mut MutationEnv<'_>, wx: i32, wy: i32) {
    let Some((min, max)) = env
        .cell
        .chunks
        .get(&(wx, wy))
        .map(|c| (c.min_level, c.max_level))
    else {
        return;
    };
    for z in min..=max {
        for m in 0..64 {
            let pos = (wx * 8 + m % 8, wy * 8 + m / 8, z);
            let n = env
                .cell
                .get_grid_square(pos.0, pos.1, pos.2)
                .map_or(0, |s| s.static_moving.len());
            for i in 0..n {
                let body = &env
                    .cell
                    .get_grid_square(pos.0, pos.1, pos.2)
                    .unwrap()
                    .static_moving[i];
                let (x, y, z) = body_xyz(body);
                let id = body.object_id.0 as i64;
                corpse_count::corpse_removed(env.cell, env.meta, x, y, z);
                let IsoCell {
                    chunks,
                    dead_bodies,
                    ..
                } = &mut *env.cell;
                let body = &mut chunks
                    .get_mut(&(wx, wy))
                    .unwrap()
                    .get_grid_square_mut(m % 8, m / 8, z)
                    .unwrap()
                    .static_moving[i];
                env.world.corpse_removed_from_world(pos, body);
                dead_bodies.remove_object(id);
                dead_bodies.remove_static_updater(id);
            }
        }
    }
    if let Some(ch) = env.cell.chunks.get_mut(&(wx, wy)) {
        corpse_count::chunk_removed_from_world(ch);
    }
}

fn body_mut(cell: &mut IsoCell, pos: SquarePos, index: usize) -> &mut IsoDeadBody {
    &mut cell
        .get_grid_square_mut(pos.0, pos.1, pos.2)
        .unwrap()
        .static_moving[index]
}

/// `isFakeDead()`: off when the sandbox disables fake dead (3) or the
/// square has stairs (both clear the flag).
fn is_fake_dead(
    env: &mut MutationEnv<'_>,
    s: &CorpseSettings,
    pos: SquarePos,
    index: usize,
) -> bool {
    let stairs = env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)
        .is_some_and(|sq| sq.has_stairs(env.sprites));
    let b = body_mut(env.cell, pos, index);
    if s.disable_fake_dead == 3 {
        b.fake_dead = false;
    }
    if stairs && b.fake_dead {
        b.fake_dead = false;
    }
    b.fake_dead
}

/// `setFakeDead(fakeDead)`.
fn set_fake_dead(
    env: &mut MutationEnv<'_>,
    s: &CorpseSettings,
    pos: SquarePos,
    index: usize,
    fake_dead: bool,
) {
    if fake_dead && s.disable_fake_dead == 3 {
        return;
    }
    let stairs = env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)
        .is_some_and(|sq| sq.has_stairs(env.sprites));
    body_mut(env.cell, pos, index).fake_dead = !stairs && fake_dead;
}

/// `getReanimateDelay()`.
pub fn reanimate_delay(s: &CorpseSettings, rand: &mut dyn crate::GameRand) -> f32 {
    let (min, mut max) = match s.reanimate {
        2 => (0.0, 0.008333334),
        3 => (0.0, 0.016666668),
        4 => (0.0, 12.0),
        5 => (48.0, 72.0),
        6 => (168.0, 336.0),
        _ => (0.0, 0.0),
    };
    if s.tutorial {
        max = 0.25;
    }
    if min == max {
        return min;
    }
    rand.next_float_range(min, max)
}

/// `setReanimateTime(hours)`: the body joins the static updaters when the
/// time is positive and leaves them otherwise.
pub fn set_reanimate_time(cell: &mut IsoCell, pos: SquarePos, index: usize, hours: f32) {
    let b = body_mut(cell, pos, index);
    b.reanimate_time = hours;
    let id = b.object_id.0 as i64;
    if hours > 0.0 {
        cell.dead_bodies.add_static_updater(id);
    } else {
        cell.dead_bodies.remove_static_updater(id);
    }
}

/// `isPlayerNearby()` (single player): a local player who sees the
/// square, alive, visible, on foot, between 2 and 4 squares away.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn is_player_nearby(
    env: &MutationEnv<'_>,
    hooks: &dyn CorpseHooks,
    pos: SquarePos,
    b: &IsoDeadBody,
) -> bool {
    for pn in 0..env.num_players {
        let can_see = hooks.can_see(pos, pn);
        if !can_see {
            continue;
        }
        let Some(p) = hooks.player(pn) else {
            continue;
        };
        if p.dead || p.ghost_or_invisible || p.in_vehicle {
            continue;
        }
        let (dx, dy) = (b.base.x - p.x, b.base.y - p.y);
        let d = dx * dx + dy * dy;
        if !(d < 4.0) && !(d > 16.0) {
            return true;
        }
    }
    false
}

/// `updateFakeDead()`: a fake dead body half an hour after death wakes up
/// when a player is near (`reanimateNow()`).
fn update_fake_dead(
    env: &mut MutationEnv<'_>,
    hooks: &dyn CorpseHooks,
    s: &CorpseSettings,
    pos: SquarePos,
    index: usize,
    world_age_hours: f64,
) -> bool {
    if !is_fake_dead(env, s, pos, index) {
        return false;
    }
    let b = body_mut(env.cell, pos, index);
    if b.is_skeleton() {
        return false;
    }
    if (b.death_time + 0.5) as f64 > world_age_hours {
        return false;
    }
    let b = &env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)
        .unwrap()
        .static_moving[index];
    if !is_player_nearby(env, hooks, pos, b) {
        return false;
    }
    if s.disable_fake_dead == 3 {
        return false;
    }
    set_reanimate_time(env.cell, pos, index, world_age_hours as f32);
    true
}

fn mod_data(b: &mut IsoDeadBody) -> &mut LuaTable {
    b.base.mod_data.get_or_insert_with(LuaTable::default)
}

/// `updateAnimalRotting(worldAge, hoursPerRotStage)`; whether the body
/// left the world (past the last stage).
fn update_animal_rotting(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    pos: SquarePos,
    index: usize,
    world_age: f32,
    hours_per_rot_stage: f32,
) -> bool {
    let b = body_mut(env.cell, pos, index);
    let age = world_age - b.death_time;
    let stages = (age / hours_per_rot_stage) as i32;
    let mut new_stage = stages.wrapping_add(b.animal_rot_stage_at_death);
    // `rawset("deathAge", (Object) age)`: a `Float`, which saving skips.
    mod_data(b).rawset(LuaKey::Str("deathAge".into()), Some(LuaValue::Unsaveable));
    if stages < 4 {
        new_stage = new_stage.clamp(-1, 4);
    }
    let CorpseVisual::Animal { rot_stage, .. } = &mut b.visual else {
        return false;
    };
    if new_stage == *rot_stage as i32 {
        return false;
    }
    if new_stage > 4 {
        remove_from_world_and_square(env, pos, index);
        return true;
    }
    *rot_stage = new_stage as i8;
    let md = mod_data(b);
    if new_stage >= 2 {
        md.rawset(LuaKey::Str("parts".into()), Some(LuaValue::Bool(false)));
        // A `Boolean`: `isSkeleton()` only takes the string "true".
        md.rawset(LuaKey::Str("skeleton".into()), Some(LuaValue::Bool(true)));
    }
    // An `Integer`, which saving skips.
    md.rawset(
        LuaKey::Str("animalRotStage".into()),
        Some(LuaValue::Unsaveable),
    );
    let b = &env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)
        .unwrap()
        .static_moving[index];
    hooks.corpse_visual_changed(pos, b);
    invalidate_body(env, pos, DIRTY_CORPSE);
    false
}

/// `Rand.Next(10) / 10` (integer division, then `float`).
fn tenth(rand: &mut dyn crate::GameRand) -> f32 {
    (rand.next(10) / 10) as f32
}

/// The maggots `updateRotting` spawns for each rot step: in the corpse
/// (`Rand.Next(chance) == 0`) and on its square (`Rand.Next(chance * 2)
/// == 0`, unless the sandbox keeps them off the ground); a zombie's are
/// poisonous.
#[allow(clippy::too_many_arguments)]
fn spawn_maggots(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    s: &CorpseSettings,
    pos: SquarePos,
    index: usize,
    chance: i32,
    was_zombie: bool,
    world_age_hours: f64,
) {
    let poison = |mut i: InventoryItem| {
        if was_zombie {
            if let ItemKind::Food(f) = &mut i.kind {
                f.poison_power = 5;
            }
        }
        i
    };
    if env.rand.next(chance) == 0 {
        if let Some(m) = hooks.create_item(MAGGOTS, env.rand) {
            let b = body_mut(env.cell, pos, index);
            if let Some(c) = b.container.as_mut() {
                let m = poison(m);
                hooks.add_to_process_items(&m);
                c.items
                    .push(pz_inventory::saved::SavedItem::Item(Box::new(m)));
            }
        }
    }
    if env.rand.next(chance * 2) != 0 || s.maggot_spawn == 2 {
        return;
    }
    let Some(m) = hooks.create_item(MAGGOTS, env.rand) else {
        return;
    };
    let x = tenth(env.rand);
    let y = tenth(env.rand);
    let item: ItemRef = std::rc::Rc::new(std::cell::RefCell::new(m));
    env.add_world_inventory_item(pos, &item, x, y, 0.0, world_age_hours);
    if was_zombie {
        if let ItemKind::Food(f) = &mut item.borrow_mut().kind {
            f.poison_power = 5;
        }
    }
}

/// `updateRotting(worldAge, hoursPerRotStage)`.
#[allow(clippy::too_many_arguments)]
fn update_rotting(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    s: &CorpseSettings,
    pos: SquarePos,
    index: usize,
    world_age: f32,
    hours_per_rot_stage: f32,
    world_age_hours: f64,
) -> UpdateBodiesOutcome {
    let b = body_mut(env.cell, pos, index);
    if b.is_skeleton() {
        return UpdateBodiesOutcome::Done;
    }
    let age = world_age - b.death_time;
    let stages = (age / hours_per_rot_stage) as i32;
    let mut new_stage = b.zombie_rot_stage_at_death.wrapping_add(stages);
    if stages < 3 {
        new_stage = new_stage.clamp(1, 3);
    }
    let was_zombie = b.was_zombie;
    let CorpseVisual::Human(v) = &mut b.visual else {
        return UpdateBodiesOutcome::Done;
    };
    if new_stage <= 3 && new_stage != v.zombie_rot_stage as i32 {
        let decay_steps = new_stage - v.zombie_rot_stage as i32;
        v.zombie_rot_stage = new_stage as i8;
        let b = &env
            .cell
            .get_grid_square(pos.0, pos.1, pos.2)
            .unwrap()
            .static_moving[index];
        hooks.corpse_visual_changed(pos, b);
        invalidate_body(env, pos, DIRTY_CORPSE);
        if env.rand.next(100) == 0 && was_zombie && s.disable_fake_dead == 2 {
            set_fake_dead(env, s, pos, index, true);
            if env.rand.next(5) == 0 {
                body_mut(env.cell, pos, index).crawling = true;
            }
        }
        if decay_steps < 1 || s.season == "Winter" || s.maggot_spawn == 3 {
            return UpdateBodiesOutcome::Done;
        }
        // As the original: `getSeasonName()` is "Early Summer" or "Late
        // Summer", never "Summer".
        let chance = if s.season == "Summer" { 3 } else { 5 };
        for _ in 0..decay_steps {
            spawn_maggots(
                env,
                hooks,
                s,
                pos,
                index,
                chance,
                was_zombie,
                world_age_hours,
            );
        }
        return UpdateBodiesOutcome::Done;
    }
    if stages == 3 && env.rand.next(7) == 0 {
        let skin = env.rand.next_range(1, 3);
        let b = body_mut(env.cell, pos, index);
        if let CorpseVisual::Human(v) = &mut b.visual {
            v.beard_model = Some(String::new());
            v.hair_model = Some(String::new());
            v.skin_texture = skin as i8;
        }
        b.was_skeleton = true;
        b.worn_items.clear();
        b.attached_items.clear();
        let Some(c) = b.container.as_mut() else {
            return UpdateBodiesOutcome::NullContainer;
        };
        c.items.clear();
        let b = &env
            .cell
            .get_grid_square(pos.0, pos.1, pos.2)
            .unwrap()
            .static_moving[index];
        hooks.corpse_visual_changed(pos, b);
        invalidate_body(env, pos, DIRTY_CORPSE);
    }
    UpdateBodiesOutcome::Done
}

/// `IsoDeadBody.updateBodies()`: every registered corpse (in
/// `ObjectIDType.DeadBody.getObjects()` order, copied first) ages by the
/// world age (`GameTime.getWorldAgeHours()`): animal corpses through four
/// rot stages, human ones through three (a fake dead one may wake up
/// instead; player corpses only when the server option says so), maggots
/// spawning as they rot, a skeleton after the third, and the corpse leaves
/// the world `HoursForCorpseRemoval` (plus a stage for a skeleton) after
/// death. Nothing with the sandbox's corpse removal off.
pub fn update_bodies(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    s: &CorpseSettings,
    world_age_hours: f64,
) -> UpdateBodiesOutcome {
    let hours = s.hours_for_corpse_removal as f32;
    if hours <= 0.0 {
        return UpdateBodiesOutcome::Done;
    }
    let mut hours_per_rot_stage = hours / 3.0;
    let world_age = world_age_hours as f32;
    let ids = env.cell.dead_bodies.ids();
    for id in ids {
        let Some((pos, index)) = env.cell.dead_bodies.index_of(env.cell, id) else {
            continue;
        };
        if matches!(
            body_mut(env.cell, pos, index).visual,
            CorpseVisual::Animal { .. }
        ) {
            hours_per_rot_stage = hours / 4.0;
            if update_animal_rotting(env, hooks, pos, index, world_age, hours_per_rot_stage) {
                continue;
            }
        }
        let b = body_mut(env.cell, pos, index);
        let CorpseVisual::Human(v) = &mut b.visual else {
            continue;
        };
        if b.death_time > world_age {
            b.death_time = world_age;
            v.zombie_rot_stage = b.zombie_rot_stage_at_death as i8;
        }
        if update_fake_dead(env, hooks, s, pos, index, world_age_hours) {
            continue;
        }
        if !s.remove_player_corpses_on_corpse_removal && !body_mut(env.cell, pos, index).was_zombie
        {
            continue;
        }
        if update_rotting(
            env,
            hooks,
            s,
            pos,
            index,
            world_age,
            hours_per_rot_stage,
            world_age_hours,
        ) == UpdateBodiesOutcome::NullContainer
        {
            return UpdateBodiesOutcome::NullContainer;
        }
        is_fake_dead(env, s, pos, index);
        let b = body_mut(env.cell, pos, index);
        let age = world_age - b.death_time;
        let extra = if b.is_skeleton() {
            hours_per_rot_stage
        } else {
            0.0
        };
        if age < hours + extra {
            continue;
        }
        remove_from_world_and_square(env, pos, index);
    }
    UpdateBodiesOutcome::Done
}

/// `IsoCell.ProcessStaticUpdaters()` for the corpses: each one's
/// `update()`, which reanimates it once its time has come.
pub fn process_static_updaters(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    world_age_hours: f64,
) {
    let mut size = env.cell.dead_bodies.static_updaters.len();
    let mut n = 0;
    while n < size {
        let id = env.cell.dead_bodies.static_updaters[n];
        update(env, hooks, id, world_age_hours);
        if size > env.cell.dead_bodies.static_updaters.len() {
            size -= 1;
        } else {
            n += 1;
        }
    }
}

/// `IsoDeadBody.update()`: a reanimation whose time has come runs.
pub fn update(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    id: i64,
    world_age_hours: f64,
) {
    let Some((pos, index)) = env.cell.dead_bodies.index_of(env.cell, id) else {
        return;
    };
    let t = body_mut(env.cell, pos, index).reanimate_time;
    if t > 0.0 && t <= world_age_hours as f32 {
        reanimate(env, hooks, pos, index);
    }
}

/// `reanimate()`: the hook makes the zombie (or animal), the body leaves
/// the world and its square, and for a human `OnContainerUpdate` fires.
pub fn reanimate(
    env: &mut MutationEnv<'_>,
    hooks: &mut dyn CorpseHooks,
    pos: SquarePos,
    index: usize,
) {
    let b = &env
        .cell
        .get_grid_square(pos.0, pos.1, pos.2)
        .unwrap()
        .static_moving[index];
    let animal = b.animal.is_some();
    if !hooks.reanimate(pos, b, env.rand) {
        return;
    }
    invalidate_body(env, pos, DIRTY_OBJECT_REMOVE);
    let Some(body) = remove_from_world_and_square(env, pos, index) else {
        return;
    };
    if !animal {
        hooks.container_update_event();
    }
    hooks.reanimated(pos, body);
}

/// `IsoGridSquare.getDeadBody()`: the first corpse on the square.
pub fn get_dead_body(cell: &IsoCell, pos: SquarePos) -> Option<&IsoDeadBody> {
    cell.get_grid_square(pos.0, pos.1, pos.2)?
        .static_moving
        .first()
}

/// `IsoGridSquare.removeCorpse(body, false)`'s Lua event on `pos`.
pub(crate) fn container_update(env: &mut MutationEnv<'_>, pos: SquarePos) {
    if let Some(sq) = env.cell.get_grid_square(pos.0, pos.1, pos.2) {
        env.world.lua_event(sq, LuaEvent::ContainerUpdate);
    }
}
