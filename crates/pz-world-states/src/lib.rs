//! Glue between the character AI states (`pz-characters`) and the loaded
//! world (`pz-world`), so neither crate depends on the other.
//!
//! [`StatesWorldAdapter`] answers the world and thumpable/door parts of
//! [`StateWorld`] from an [`IsoCell`]:
//!
//! - squares (`getGridSquare`, rooms, buildings, smoke, sprite names, the
//!   sneak modifier, `isSomethingTo`, `isFree`, stairs, chunks, sheet
//!   ropes, corpses) and `LosUtil` (`lineClear`, `lineClearCollide`);
//! - vehicles from the chunks' saved `BaseVehicle`s (`getIntersectPoint`,
//!   `getClosestPointOnPoly`);
//! - `IsoGridSquare.testCollideSpecialObjects`, and the thump code of
//!   `IsoDoor`, `IsoThumpable`, `IsoWindow`, `IsoWindowFrame`,
//!   `IsoBarricade` and plain `IsoObject`s (`getThumpableFor`,
//!   `getThumpCondition`, `Thump`, `Damage`, `destroy`, window smashing),
//!   which change the cell's objects.
//!
//! What lives outside the world (random numbers, other characters, sounds,
//! items, Lua, global object systems, path finding, lighting, door
//! toggling) comes from a [`StatesEngine`] the caller supplies. Character,
//! path-finding, lighting and audio methods of [`StateWorld`] keep their
//! defaults; the engine wraps this adapter for those.

mod collide;
mod query;
mod thump;
mod vehicles;

pub use collide::{CollideMoverInfo, NewRenderEffect, RenderEffectType, StatesFire, WorldView};
pub use query::Env;
pub use vehicles::{VehicleBox, VehicleGeometry};

use std::collections::HashMap;

use pz_characters::character::state::{
    CorpseInfo, EatTarget, SquareInfo, StateWorld, ThumpInfo, ThumpTarget,
};
use pz_characters::GameCharacter;
use pz_pathfind::mover::{CharacterView, MoverId, ObjectRef};
use pz_world::java::{fastfloor, GameRand};
use pz_world::los::{self, TestResults};
use pz_world::objects::IsoObject;
use pz_world::square::{EdgeFacing, HoppableAction};
use pz_world::{
    BuildingIdx, IsoCell, IsoDirections, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, VisionEnv,
};

/// What a thumper or a character the thump code looks at is, as the world
/// code reads it (`IsoGameCharacter` / `IsoZombie` fields).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thumper {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getCurrentSquare()`.
    pub current_square: Option<(i32, i32, i32)>,
    /// `instanceof IsoZombie`.
    pub zombie: bool,
    /// `instanceof IsoAnimal`.
    pub animal: bool,
    /// `isCrawling()`.
    pub crawling: bool,
    /// `IsoZombie.strength`.
    pub strength: i32,
    /// `IsoZombie.cognition`.
    pub cognition: i32,
}

impl Thumper {
    /// The thumper view of a character.
    pub fn of<I: 'static>(chr: &GameCharacter<I>) -> Self {
        Thumper {
            id: chr.id,
            x: chr.vars.x,
            y: chr.vars.y,
            z: chr.vars.z,
            current_square: chr.current_square_xyz(),
            zombie: chr.is_zombie(),
            animal: chr.is_animal(),
            crawling: chr.vars.crawling,
            strength: chr.zombie.strength,
            cognition: chr.zombie.cognition,
        }
    }
}

/// Sandbox and game-mode values the thump code reads (runtime input).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumpSettings {
    /// `SandboxOptions.lore.strength` (1 is superhuman).
    pub lore_strength: i32,
    /// `SandboxOptions.lore.thumpOnConstruction`.
    pub thump_on_construction: bool,
    /// `SandboxOptions.lore.triggerHouseAlarm`.
    pub trigger_house_alarm: bool,
    /// `Core.gameMode.equals("LastStand")`.
    pub last_stand: bool,
    /// `Core.gameMode.equals("Tutorial")` / `Core.tutorial`.
    pub tutorial: bool,
    /// `GameClient.client`: thumps are only reported
    /// ([`StatesEngine::client_thump_hit`]).
    pub client: bool,
    /// `GameServer.server`: no render effects.
    pub server: bool,
}

impl Default for ThumpSettings {
    fn default() -> Self {
        ThumpSettings {
            lore_strength: 2,
            thump_on_construction: true,
            trigger_house_alarm: false,
            last_stand: false,
            tutorial: false,
            client: false,
            server: false,
        }
    }
}

/// Runtime object state the original keeps on objects but does not save:
/// `IsoThumpable.partialThumpDmg`, `IsoObject.partialThumpDmg` and
/// `IsoObject.damage`, and the corpses' `eatingZombies`. Keep one per
/// world and hand it to every adapter.
#[derive(Clone, Debug, Default)]
pub struct StatesRuntime {
    /// `partialThumpDmg` by object.
    pub partial_thump_dmg: HashMap<ObjectRef, f32>,
    /// `IsoObject.damage` of plain objects (100 when absent).
    pub object_damage: HashMap<ObjectRef, i32>,
    /// `IsoDeadBody.eatingZombies` by corpse id.
    pub eating: HashMap<i64, Vec<i32>>,
}

impl StatesRuntime {
    /// Drops the state of an object that left the world.
    pub fn forget(&mut self, r: ObjectRef) {
        self.partial_thump_dmg.remove(&r);
        self.object_damage.remove(&r);
    }
}

/// `WorldSoundManager.instance.addSound(source, x, y, z, radius, volume,
/// stressHumans, zombieIgnoreDist, stressMod)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSoundRequest {
    /// The thumper's id, or `None` for `null`.
    pub source: Option<i32>,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    pub volume: i32,
    pub stress_humans: bool,
    pub zombie_ignore_dist: f32,
    pub stress_mod: f32,
}

/// What the states' world code needs from outside `pz-world`. Every
/// method has the answer of an engine with nothing in it; the provider is
/// named on each.
pub trait StatesEngine {
    /// `Rand` (`zombie.core.random.Rand`): the game's random source.
    fn rand(&mut self) -> &mut dyn GameRand;

    // ---- characters (pz-characters' owner) ----

    /// A character by id, as the thump code reads it.
    fn character(&self, _id: i32) -> Option<Thumper> {
        None
    }
    /// `getSurroundingThumpers()` of `chr`: the cell's zombies within 1.5
    /// of it (`chr.getCurrentSquare() == null` gives 1).
    fn surrounding_thumpers(&self, _chr: &Thumper) -> i32 {
        1
    }
    /// The non-ghost players standing on `sq` (`getMovingObjects()`).
    fn players_on_square(&self, _sq: (i32, i32, i32)) -> Vec<i32> {
        Vec::new()
    }
    /// `!square.getMovingObjects().isEmpty()`.
    fn has_moving_objects(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// A character's `getEatingZombies()`: add or remove `zombie`.
    fn set_character_eating(&mut self, _chr: i32, _zombie: i32, _eating: bool) {}
    /// A character's `getEatingZombies().size()`.
    fn character_eating_count(&self, _chr: i32) -> usize {
        0
    }
    /// `IsoWindow.getFirstCharacterClosing()` has `CloseWindowOutcome` of
    /// `success`.
    fn window_closing_success(&self, _window: ObjectRef) -> bool {
        false
    }

    // ---- lighting (pz-lighting) ----

    /// `square.isCanSee(playerIndex)` (`lighting[playerIndex].bCanSee()`).
    fn can_see(&self, _sq: (i32, i32, i32), _player_index: i32) -> bool {
        false
    }
    /// `square.isCouldSee(playerIndex)`.
    fn could_see(&self, _sq: (i32, i32, i32), _player_index: i32) -> bool {
        false
    }

    // ---- global object systems (pz-lua-api's `global_objects`) ----

    /// `square.hasFarmingPlant()`.
    fn has_farming_plant(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.destroyFarmingPlant()`.
    fn destroy_farming_plant(&mut self, _sq: (i32, i32, i32)) {}
    /// `square.hasLitCampfire()` (the `campfire` system's `isLit`).
    fn has_lit_campfire(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.putOutCampfire()`.
    fn put_out_campfire(&mut self, _sq: (i32, i32, i32)) {}

    // ---- path finding (pz-pathfind) ----

    /// `PolygonalMap2.instance.canStandAt(x, y, z, null, flags)`.
    fn can_stand_at(&self, _x: f32, _y: f32, _z: i32, _flags: i32) -> bool {
        true
    }
    /// `PolygonalMap2.instance.squareChanged(sq)` and the other "square
    /// changed" notices (`MapCollisionData`, `IsoRegions`, light and
    /// render invalidation) after the adapter changed a square's objects.
    fn square_changed(&mut self, _sq: (i32, i32, i32)) {}

    // ---- audio (pz-audio) ----

    /// `WorldSoundManager.instance.addSound(...)`.
    fn add_world_sound(&mut self, _sound: WorldSoundRequest) {}
    /// `chr.getEmitter().playSound(name, object)`.
    fn play_sound(&mut self, _chr: i32, _name: &str, _object: Option<ObjectRef>) {}
    /// `SoundManager.instance.PlayWorldSound(name, square, ...)`.
    fn play_world_sound(&mut self, _name: &str, _sq: (i32, i32, i32)) {}
    /// The break sound of a thumpable (`IsoThumpable.getBreakSound()`:
    /// the Lua-set `breakSound`, else `GetBreakFurnitureSound(sprite)`).
    fn thumpable_break_sound(&self, _obj: &IsoObject) -> String {
        "BreakFurniture".into()
    }

    // ---- items, Lua and the rest of the object code ----

    /// `square.AddWorldInventoryItem(InventoryItemFactory.CreateItem(item),
    /// x, y, 0)`, with `setKeyId(key_id)` when given.
    fn add_world_item(
        &mut self,
        _sq: (i32, i32, i32),
        _item: &str,
        _x: f32,
        _y: f32,
        _key_id: Option<i32>,
    ) {
    }
    /// `dumpContentsInSquare()` / the containers' items dropped on the
    /// square, for `obj` just before it is removed.
    fn dump_contents(&mut self, _sq: (i32, i32, i32), _obj: &IsoObject) {}
    /// `obj` left `sq` (`OnObjectAboutToBeRemoved` / `OnTileRemoved`, and
    /// `OnDestroyIsoThumpable` for destroyed thumpables when `destroyed`).
    fn object_removed(&mut self, _sq: (i32, i32, i32), _obj: &IsoObject, _destroyed: bool) {}
    /// `LuaEventManager.triggerEvent("OnContainerUpdate")`.
    fn on_container_update(&mut self) {}
    /// `building.forceAwake()`.
    fn force_awake(&mut self, _building: BuildingIdx) {}
    /// `AmbientStreamManager.instance.doAlarm(def)` for an alarmed building.
    fn do_alarm(&mut self, _building: BuildingIdx) {}
    /// `IsoDoor/IsoThumpable.ToggleDoor(chr)` on the cell (the door code
    /// with its sounds, Lua and path invalidation).
    fn toggle_door(&mut self, _cell: &mut IsoCell, _door: ObjectRef, _chr: i32) {}
    /// `IsoWindow.ToggleWindow(chr)` on the cell.
    fn toggle_window(&mut self, _cell: &mut IsoCell, _window: ObjectRef, _chr: i32) {}
    /// `BrokenFences.getInstance().isBreakableObject(obj)` (the sprite
    /// lists Lua registers).
    fn is_breakable_fence(&self, _sprite_name: &str) -> bool {
        false
    }
    /// `BrokenFences.getInstance().destroyFence(obj, dir)` on the cell.
    fn destroy_fence(&mut self, _cell: &mut IsoCell, _obj: ObjectRef, _dir: IsoDirections) {}
    /// `GameClient.connection.thumpHits.put(zombie, thumpable)`.
    fn client_thump_hit(&mut self, _zombie: i32, _target: ThumpTarget) {}

    // ---- collision, hoppables and fire (the character owner) ----

    /// The mover-aware collision view of a character (its collided flags,
    /// states and `shouldIgnoreCollisionWithSquare` squares, made with
    /// [`CollideMoverInfo::of`]). `None` falls back to [`Self::character`]
    /// and the kind in the `MoverId`.
    fn collide_mover(&self, _id: i32) -> Option<CollideMoverInfo> {
        None
    }
    /// Apply what `checkBreakHoppable`, `checkHitHoppable` and
    /// `checkBreakBendableFence` ask of `chr`, in order
    /// (`setThumpTarget(obj)`, `climbOverFence(dir)`).
    fn hoppable_actions(&mut self, _chr: i32, _actions: &[HoppableAction]) {}
    /// `BentFences.getInstance().isEnabled()`.
    fn bent_fences_enabled(&self) -> bool {
        false
    }
    /// `BentFences.getInstance().isUnbentObject(obj, facing)` (the sprite
    /// lists Lua registers).
    fn is_unbent_fence(&self, _sprite_name: &str, _facing: EdgeFacing) -> bool {
        false
    }
    /// `IsoFireManager` and the fire hooks for `StateWorld::character_fire`
    /// (`None`: no fire simulation).
    fn fire(&mut self) -> Option<StatesFire<'_>> {
        None
    }

    // ---- render effects and tree sounds (the renderer, pz-audio) ----

    /// `doTreeNoises()` of `chr` on its current square (`treeSoundMgr
    /// .addSquare` when the listener is within 20; no draws).
    fn do_tree_noises(&mut self, _chr: i32, _sq: (i32, i32, i32)) {}
    /// `getRenderEffectMaster().objectRenderEffects != null`.
    fn has_render_effect(&self, _obj: ObjectRef) -> bool {
        false
    }
    /// `objectRenderEffects = ObjectRenderEffects.getNew(obj, type, false)`.
    fn set_render_effect(&mut self, _obj: ObjectRef, _effect: NewRenderEffect) {}

    // ---- path finding host (pz-pathfind's `PathHost`) ----

    /// `PolygonalMap2.instance.resolveCollision(chr, nx, ny)`
    /// (`PathHost::resolve_collision` over `world`).
    fn resolve_collision(
        &mut self,
        _world: WorldView<'_>,
        _chr: &dyn CharacterView,
        nx: f32,
        ny: f32,
    ) -> (f32, f32) {
        (nx, ny)
    }
    /// `PolygonalMap2.instance.resolveCollision(chr, radius, finalPos)`
    /// (`PathHost::resolve_collision_radius` over `world`).
    fn resolve_collision_radius(
        &mut self,
        _world: WorldView<'_>,
        chr: &dyn CharacterView,
        _radius: f32,
    ) -> (bool, (f32, f32)) {
        (false, (chr.x(), chr.y()))
    }
    /// `PathfindNative.cancelRequest(mover)`
    /// (`PathfindService::cancel_request`).
    fn path_cancel_request(&mut self, _mover: MoverId) {}

    // ---- vehicles (the vehicle simulation) ----

    /// The script box of a vehicle script (`getExtents()`,
    /// `getCenterOfMassOffset()`), `None` when the script is missing.
    fn vehicle_box(&self, _script_name: &str) -> Option<VehicleBox> {
        None
    }
    /// What the thump code reads about a vehicle target.
    fn vehicle_thump_info(&self, _vehicle: i16, _chr: i32) -> Option<ThumpInfo> {
        None
    }
    /// `BaseVehicle.Thump(chr)`.
    fn vehicle_thump(&mut self, _vehicle: i16, _chr: &Thumper) {}
}

/// [`StateWorld`] over the loaded cell.
pub struct StatesWorldAdapter<'a> {
    pub cell: &'a mut IsoCell,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a mut IsoSpriteManager,
    pub settings: ThumpSettings,
    pub runtime: &'a mut StatesRuntime,
    pub engine: &'a mut dyn StatesEngine,
}

impl<'a> StatesWorldAdapter<'a> {
    pub fn new(
        cell: &'a mut IsoCell,
        meta: &'a IsoMetaGrid,
        sprites: &'a mut IsoSpriteManager,
        settings: ThumpSettings,
        runtime: &'a mut StatesRuntime,
        engine: &'a mut dyn StatesEngine,
    ) -> Self {
        StatesWorldAdapter {
            cell,
            meta,
            sprites,
            settings,
            runtime,
            engine,
        }
    }

    /// The read-only queries over the cell.
    pub fn env(&self) -> Env<'_> {
        Env::new(self.cell, self.sprites, self.meta)
    }

    fn at(&self, p: (i32, i32, i32)) -> Option<&IsoGridSquare> {
        self.cell.get_grid_square(p.0, p.1, p.2)
    }

    /// `square.getBuilding()`: the building of the square's room.
    pub fn square_building_idx(&self, sq: &IsoGridSquare) -> Option<BuildingIdx> {
        sq.room_def(self.meta).and_then(|r| r.building)
    }

    /// `testCollideSpecialObjects` between two squares, as an object ref.
    pub fn special_object_ref(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> Option<ObjectRef> {
        let env = self.env();
        let this = env.at(from)?;
        let (sq, id) = env.test_collide_special_objects(this, env.at(to))?;
        Some(ObjectRef {
            x: sq.x,
            y: sq.y,
            z: sq.z,
            id,
        })
    }

    fn vision_line(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        ignore_doors: bool,
    ) -> TestResults {
        let hooks = self.env().hooks();
        let env = VisionEnv::new(&*self.cell, self.sprites, &hooks);
        los::line_clear(&env, from.0, from.1, from.2, to.0, to.1, to.2, ignore_doors)
    }

    /// `LosUtil.lineClearCollide(x1, y1, z1, x0, y0, z0, bIgnoreDoors)`
    /// with `from` as `(x1, y1, z1)`: true when blocked.
    pub fn line_clear_collide_squares(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        ignore_doors: bool,
    ) -> bool {
        let env = self.env();
        let hooks = env.hooks();
        let (x1, y1, z1) = from;
        let (mut x0, mut y0, mut z0) = to;
        let (mut dx, mut dy, mut dz) = (x1 - x0, y1 - y0, z1 - z0);
        let mut t = 0.5f32;
        let mut t2 = 0.5f32;
        let mut b = env.sq(x0, y0, z0);
        let step = |a: Option<&IsoGridSquare>, b: Option<&IsoGridSquare>, doors: bool| -> bool {
            let (Some(a), Some(b)) = (a, b) else {
                return false;
            };
            let mut blocked =
                a.calculate_collide(Some(b), false, false, true, true, self.sprites, &env);
            if doors && !ignore_doors && a.is_door_blocked_to(Some(b), &env, &hooks) {
                blocked = true;
            }
            blocked
        };
        if dx.abs() > dy.abs() && dx.abs() > dz.abs() {
            let mut m = dy as f32 / dx as f32;
            let mut m2 = dz as f32 / dx as f32;
            t += y0 as f32;
            t2 += z0 as f32;
            dx = if dx < 0 { -1 } else { 1 };
            m *= dx as f32;
            m2 *= dx as f32;
            while x0 != x1 {
                x0 += dx;
                t += m;
                t2 += m2;
                let a = env.sq(x0, fastfloor(t), fastfloor(t2));
                if step(a, b, true) {
                    return true;
                }
                b = a;
            }
        } else if dy.abs() >= dx.abs() && dy.abs() > dz.abs() {
            let mut m = dx as f32 / dy as f32;
            let mut m2 = dz as f32 / dy as f32;
            t += x0 as f32;
            t2 += z0 as f32;
            dy = if dy < 0 { -1 } else { 1 };
            m *= dy as f32;
            m2 *= dy as f32;
            while y0 != y1 {
                y0 += dy;
                t += m;
                t2 += m2;
                let a = env.sq(fastfloor(t), y0, fastfloor(t2));
                if step(a, b, true) {
                    return true;
                }
                b = a;
            }
        } else {
            let mut m = dx as f32 / dz as f32;
            let mut m2 = dy as f32 / dz as f32;
            t += x0 as f32;
            t2 += y0 as f32;
            dz = if dz < 0 { -1 } else { 1 };
            m *= dz as f32;
            m2 *= dz as f32;
            while z0 != z1 {
                z0 += dz;
                t += m;
                t2 += m2;
                let a = env.sq(fastfloor(t), fastfloor(t2), z0);
                if step(a, b, false) {
                    return true;
                }
                b = a;
            }
        }
        false
    }

    /// The corpse with `id` (see [`corpse_id`]).
    fn corpse(&self, id: i64) -> Option<(&IsoGridSquare, usize)> {
        let (x, y, z, i) = corpse_pos(id);
        let sq = self.cell.get_grid_square(x, y, z)?;
        (i < sq.static_moving.len()).then_some((sq, i))
    }

    fn corpse_info(&self, sq: &IsoGridSquare, i: usize) -> CorpseInfo {
        let b = &sq.static_moving[i];
        let id = corpse_id(sq.x, sq.y, sq.z, i);
        let animal = b.animal.as_ref().is_some_and(|a| !a.animal_type.is_empty());
        let skel = b
            .base
            .mod_data
            .as_ref()
            .and_then(|t| t.rawget(&pz_world::save::lua_table::LuaKey::Str("skeleton".into())));
        let animal_skeleton = matches!(skel, Some(pz_world::save::lua_table::LuaValue::Str(s)) if s == "true")
            || matches!(skel, Some(pz_world::save::lua_table::LuaValue::Bool(true)));
        CorpseInfo {
            id,
            x: b.base.x,
            y: b.base.y,
            z: b.base.z,
            skeleton: b.is_skeleton(),
            zombie: b.was_zombie,
            animal,
            animal_skeleton,
            in_world: true,
            eating: self.runtime.eating.get(&id).map_or(0, Vec::len),
        }
    }
}

/// The id the adapter gives the corpse at `static_moving[index]` of square
/// `(x, y, z)`: corpses do not move, so the place names them.
pub fn corpse_id(x: i32, y: i32, z: i32, index: usize) -> i64 {
    ((x as i64 & 0xFF_FFFF) << 40)
        | ((y as i64 & 0xFF_FFFF) << 16)
        | ((z as i64 & 0xFF) << 8)
        | (index as i64 & 0xFF)
}

/// The square and list index of a [`corpse_id`].
pub fn corpse_pos(id: i64) -> (i32, i32, i32, usize) {
    let sext = |v: i64, bits: u32| ((v << (64 - bits)) >> (64 - bits)) as i32;
    (
        sext(id >> 40, 24),
        sext((id >> 16) & 0xFF_FFFF, 24),
        sext((id >> 8) & 0xFF, 8),
        (id & 0xFF) as usize,
    )
}

impl<I: 'static> StateWorld<I> for StatesWorldAdapter<'_> {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        let sq = self.cell.get_grid_square(x, y, z)?;
        let m = &*self.sprites;
        Some(SquareInfo {
            x,
            y,
            z,
            in_room: sq.room_id != -1,
            solid_floor: sq.treat_as_solid_floor(m),
            has_stairs: sq.has_stairs(m),
            has_stairs_north: sq.has_stairs_north(m),
            has_stairs_west: sq.has_stairs_west(m),
        })
    }

    fn square_room(&self, sq: (i32, i32, i32)) -> Option<i64> {
        self.at(sq).filter(|s| s.room_id != -1).map(|s| s.room_id)
    }

    fn square_building(&self, sq: (i32, i32, i32)) -> Option<i64> {
        let s = self.at(sq)?;
        self.square_building_idx(s).map(|b| b.0 as i64)
    }

    fn square_smoke(&self, sq: (i32, i32, i32)) -> bool {
        self.at(sq).is_some_and(|s| self.env().sq_has(s, "smoke"))
    }

    fn square_can_see(&self, sq: (i32, i32, i32), player_index: i32) -> bool {
        self.at(sq).is_some() && self.engine.can_see(sq, player_index)
    }

    fn square_could_see(&self, sq: (i32, i32, i32), player_index: i32) -> bool {
        self.at(sq).is_some() && self.engine.could_see(sq, player_index)
    }

    fn square_sprite_names(&self, sq: (i32, i32, i32)) -> Vec<String> {
        let env = self.env();
        self.at(sq)
            .map(|s| {
                s.objects
                    .iter()
                    .filter_map(|o| env.sprite_name(o))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn grid_sneak_modifier(&self, sq: (i32, i32, i32), only_solid_trans: bool) -> f32 {
        self.at(sq)
            .map_or(1.0, |s| self.env().grid_sneak_modifier(s, only_solid_trans))
    }

    fn is_something_to(&self, a: (i32, i32, i32), b: (i32, i32, i32)) -> bool {
        match (self.at(a), self.at(b)) {
            (Some(a), Some(b)) => self.env().is_something_to(a, b),
            _ => false,
        }
    }

    /// `isFree(true)`.
    fn square_is_free(&self, sq: (i32, i32, i32)) -> bool {
        let Some(s) = self.at(sq) else { return false };
        !self.engine.has_moving_objects(sq) && self.env().is_free(s)
    }

    fn same_staircase(&self, sq: (i32, i32, i32), x: i32, y: i32, z: i32) -> bool {
        self.at(sq)
            .is_some_and(|s| self.env().is_same_staircase(s, x, y, z))
    }

    fn vehicle_between(&self, from: (f32, f32, f32), to: (f32, f32, f32)) -> bool {
        vehicles::vehicle_between(self.cell, &*self.engine, from, to)
    }

    fn vehicle_dist_squared(&self, vehicle: i16, x: f32, y: f32) -> f32 {
        vehicles::vehicle_dist_squared(self.cell, &*self.engine, vehicle, x, y)
    }

    fn chunk_loaded(&self, wx: i32, wy: i32) -> bool {
        self.cell.chunks.contains_key(&(wx, wy))
    }

    fn los_line_clear(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        ignore_doors: bool,
    ) -> TestResults {
        self.vision_line(from, to, ignore_doors)
    }

    fn los_line_clear_collide(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        ignore_doors: bool,
    ) -> bool {
        self.line_clear_collide_squares(from, to, ignore_doors)
    }

    fn special_object_between(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> Option<ThumpTarget> {
        self.special_object_ref(from, to).map(ThumpTarget::Object)
    }

    fn thump_info(&self, target: ThumpTarget, chr: i32) -> Option<ThumpInfo> {
        match target {
            ThumpTarget::Object(r) => {
                self.object_thump_info(r, self.engine.character(chr).as_ref())
            }
            ThumpTarget::Vehicle(v) => self.engine.vehicle_thump_info(v, chr),
        }
    }

    fn thump(&mut self, target: ThumpTarget, chr: &mut GameCharacter<I>) {
        let t = Thumper::of(chr);
        let mut new_target = Some(chr.zombie.thump_target);
        match target {
            ThumpTarget::Object(r) => self.thump_object(r, &t, &mut new_target),
            ThumpTarget::Vehicle(v) => self.engine.vehicle_thump(v, &t),
        }
        if let Some(nt) = new_target {
            chr.zombie.thump_target = nt;
        }
    }

    /// `IsoZombie.damageSheetRope`'s test: a `climbSheetN/E/S/W` flag and
    /// `getSheetRope() != null`.
    fn has_sheet_rope(&self, sq: (i32, i32, i32)) -> bool {
        self.at(sq).is_some_and(|s| {
            let env = self.env();
            ["climbSheetN", "climbSheetE", "climbSheetS", "climbSheetW"]
                .into_iter()
                .any(|n| env.sq_has(s, n))
                && env.sheet_rope_index(s).is_some()
        })
    }

    fn damage_sheet_rope(&mut self, sq: (i32, i32, i32), damage: f32) {
        self.damage_sheet_rope_at(sq, damage);
    }

    fn has_farming_plant(&self, sq: (i32, i32, i32)) -> bool {
        self.at(sq).is_some() && self.engine.has_farming_plant(sq)
    }

    fn destroy_farming_plant(&mut self, sq: (i32, i32, i32)) {
        if self.at(sq).is_some() {
            self.engine.destroy_farming_plant(sq);
        }
    }

    fn has_lit_campfire(&self, sq: (i32, i32, i32)) -> bool {
        self.at(sq).is_some() && self.engine.has_lit_campfire(sq)
    }

    fn put_out_campfire(&mut self, sq: (i32, i32, i32)) {
        if self.at(sq).is_some() {
            self.engine.put_out_campfire(sq);
        }
    }

    /// `square.testCollideAdjacent(mover, dx, dy, dz)` with the mover from
    /// [`StatesEngine::collide_mover`]; the special object it hits is
    /// `special_object_between(from, from + d)` (mover independent).
    fn test_collide_adjacent(
        &self,
        mover: MoverId,
        from: (i32, i32, i32),
        dx: i32,
        dy: i32,
        dz: i32,
    ) -> bool {
        let m = self.collide_mover_info(mover);
        self.test_collide_adjacent_mover(&m, from, (dx, dy, dz))
            .is_none_or(|r| r.blocked)
    }

    /// `testCollideAdjacent(mover, ...)` with the character's own view.
    fn test_collide_adjacent_mover(
        &self,
        mover: &dyn pz_world::square::CollideMover,
        _id: MoverId,
        from: (i32, i32, i32),
        dx: i32,
        dy: i32,
        dz: i32,
    ) -> pz_world::square::CollideAdjacent {
        let Some(sq) = self.cell.get_grid_square(from.0, from.1, from.2) else {
            return pz_world::square::CollideAdjacent {
                blocked: true,
                ..Default::default()
            };
        };
        self.collide_env()
            .test_collide_adjacent(sq, Some(mover), dx, dy, dz)
    }

    fn hoppable_actions(
        &self,
        mover: &dyn pz_world::square::CollideMover,
        chr: i32,
        last: (i32, i32, i32),
    ) -> Vec<HoppableAction> {
        self.hoppable_actions_dyn(mover, chr, last)
    }

    fn fence_climb(
        &mut self,
        chr: &dyn CharacterView,
        zombie: bool,
        from: (i32, i32, i32),
        dir: IsoDirections,
    ) -> Option<pz_characters::character::state::FenceClimb> {
        let t = Thumper {
            x: chr.x(),
            y: chr.y(),
            z: chr.z(),
            current_square: Some(from),
            zombie,
            ..Default::default()
        };
        self.fence_climb_for(&t, from, dir)
    }

    /// `checkBreakHoppable()`, `checkHitHoppable()` and
    /// `checkBreakBendableFence(current)`, handed to
    /// [`StatesEngine::hoppable_actions`].
    fn collided_with_hoppables(&mut self, chr: i32, current: Option<(i32, i32, i32)>) {
        let Some(last) = current else { return };
        let Some(m) = self.engine.collide_mover(chr) else {
            return;
        };
        let actions = self.hoppable_actions_for(&m, last);
        if !actions.is_empty() {
            self.engine.hoppable_actions(chr, &actions);
        }
    }

    /// `doTreeNoises()` and `setRenderEffect(Vegetation_Rustle)` on the
    /// square's trees and bushes past its first object; a new effect draws
    /// `Rand.Next(45f, 60f)`.
    fn rustle_vegetation(
        &mut self,
        chr: i32,
        sq: (i32, i32, i32),
        ctx: &dyn pz_characters::character::SimContext,
    ) {
        let targets = self.rustle_targets(sq);
        let has_tree_or_bush = !targets.is_empty() || {
            // A tree or bush that is the square's first object still makes
            // the noise.
            let env = self.env();
            env.at(sq).is_some_and(|s| {
                s.has_type(self.sprites.tables.ty.tree)
                    || s.objects.iter().any(|o| {
                        o.sprite.is_some_and(|i| {
                            let spr = &self.sprites.sprites[i];
                            spr.tileset_name.as_deref() == Some("f_bushes_1")
                                || spr.properties.get(&self.sprites.alias, "Bush").is_some()
                        })
                    })
            })
        };
        if !has_tree_or_bush {
            return;
        }
        if !self.settings.server {
            self.engine.do_tree_noises(chr, sq);
        }
        if self.settings.server {
            return;
        }
        for (r, is_tree, is_big) in targets {
            if self.engine.has_render_effect(r) {
                continue;
            }
            let total_max_time = ctx.rand_next_float(45.0, 60.0);
            self.engine.set_render_effect(
                r,
                NewRenderEffect {
                    kind: RenderEffectType::VegetationRustle,
                    total_max_time,
                    is_tree,
                    is_big: is_tree && is_big,
                },
            );
        }
    }

    /// `FireCheck()`, `SpreadFire()`, `ReduceHealthWhenBurning()` with the
    /// engine's `IsoFireManager` ([`StatesEngine::fire`]) on this cell.
    fn character_fire(
        &mut self,
        chr: &mut dyn pz_world::fire::character::FireCharacter,
        frame: pz_world::fire::character::CharacterFireFrame,
        rand: &mut dyn GameRand,
    ) {
        let Some(f) = self.engine.fire() else { return };
        let mut w = pz_world::fire::manager::FireWorld {
            cell: &mut *self.cell,
            meta: self.meta,
            sprites: &mut *self.sprites,
            flags: f.flags,
            settings: f.settings,
            hooks: f.hooks,
        };
        pz_characters::character::fire::update_character_fire(f.manager, &mut w, chr, frame, rand);
    }

    fn resolve_collision(&mut self, chr: &dyn CharacterView, nx: f32, ny: f32) -> (f32, f32) {
        let view = WorldView {
            cell: &*self.cell,
            meta: self.meta,
            sprites: &*self.sprites,
        };
        self.engine.resolve_collision(view, chr, nx, ny)
    }

    fn resolve_collision_radius(
        &mut self,
        chr: &dyn CharacterView,
        radius: f32,
    ) -> (bool, (f32, f32)) {
        let view = WorldView {
            cell: &*self.cell,
            meta: self.meta,
            sprites: &*self.sprites,
        };
        self.engine.resolve_collision_radius(view, chr, radius)
    }

    fn path_cancel_request(&mut self, chr: &mut dyn pz_pathfind::mover::PathMover) {
        self.engine.path_cancel_request(chr.mover_id());
    }

    fn players_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
        if self.at(sq).is_none() {
            return Vec::new();
        }
        self.engine.players_on_square(sq)
    }

    /// `square.getDeadBody()`: the first corpse in `staticMovingObjects`.
    fn dead_body_at(&self, sq: (i32, i32, i32)) -> Option<CorpseInfo> {
        let s = self.at(sq)?;
        (!s.static_moving.is_empty()).then(|| self.corpse_info(s, 0))
    }

    fn dead_body(&self, id: i64) -> Option<CorpseInfo> {
        let (sq, i) = self.corpse(id)?;
        Some(self.corpse_info(sq, i))
    }

    fn set_eating(&mut self, target: EatTarget, zombie: i32, eating: bool) {
        match target {
            EatTarget::Body(b) => {
                let list = self.runtime.eating.entry(b).or_default();
                if eating {
                    if !list.contains(&zombie) {
                        list.push(zombie);
                    }
                } else {
                    list.retain(|&z| z != zombie);
                }
            }
            EatTarget::Character(c) => self.engine.set_character_eating(c, zombie, eating),
        }
    }

    fn eating_count(&self, target: EatTarget) -> usize {
        match target {
            EatTarget::Body(b) => self.runtime.eating.get(&b).map_or(0, Vec::len),
            EatTarget::Character(c) => self.engine.character_eating_count(c),
        }
    }
}

#[cfg(test)]
mod tests;
