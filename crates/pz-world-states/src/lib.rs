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
//!
//! The adapter is also the population manager's [`SittingChecks`]
//! (`pz-world-popman`), and [`population_host`] runs that manager over the
//! adapter `RealZombies` holds ([`population`]).
//!
//! [`SittingChecks`]: pz_world_popman::SittingChecks

mod bent_fences;
mod broken_fences;
mod climb;
mod collide;
mod context;
mod door;
mod door_hits;
mod floor;
pub mod population;
mod query;
mod thump;
mod vehicles;
mod windows;

pub use bent_fences::{BentFenceEntry, BentFences, FenceTileDef, ThumpData};
pub use broken_fences::{
    AdjacentDamage, BrokenFences, BrokenTile, DebrisItem, FenceRand, UnknownTile,
};
pub use collide::{CollideMoverInfo, NewRenderEffect, RenderEffectType, StatesFire, WorldView};
pub use population::{population_host, population_view, real_zombie_on_square};
pub use query::Env;
pub use vehicles::{VehicleBox, VehicleGeometry};

use std::collections::HashMap;

use pz_characters::character::state::{
    CorpseInfo, EatTarget, SheetRopeExit, SheetRopeSquare, SquareInfo, StateWorld, ThumpInfo,
    ThumpTarget, WindowInfo,
};
use pz_characters::character::player::context::{ContextQuery, ContextSpecial};
use pz_characters::character::player::ContextualAction;
use pz_characters::GameCharacter;
use pz_pathfind::behavior::{BehaviorResult, CharacterLookup, PathFindBehavior2};
use pz_pathfind::host::{PathEnv, PathHost};
use pz_pathfind::mover::{CharacterView, MoverId, ObjectRef, PathMover, PathWorldHooks};
use pz_world::java::{fastfloor, GameRand};
use pz_world::los::{self, TestResults};
use pz_world::object_slot::IsoObjectRef;
use pz_world::objects::IsoObject;
use pz_world::square::{EdgeFacing, HoppableAction};
use pz_world::{
    BuildingIdx, IsoCell, IsoDirections, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, SquareRef,
    VisionEnv,
};

/// A character already in CloseWindowState at the start of a world query.
#[derive(Clone, Copy)]
pub struct ClosingWindow {
    pub actor: i32,
    pub target: ObjectRef,
    pub success: bool,
}

impl ClosingWindow {
    pub fn of<I: 'static>(chr: &GameCharacter<I>) -> Option<Self> {
        let target = pz_characters::character::states::window::closing_window(chr)?;
        Some(Self { actor: chr.id, target, success: chr.variable_string("CloseWindowOutcome").eq_ignore_ascii_case("success") })
    }
}

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
    door_hits: Vec<door_hits::DoorHit>,
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

pub use pz_characters::character::context::WorldSoundRequest;

/// What the states' world code needs from outside `pz-world`. Every
/// method has the answer of an engine with nothing in it; the provider is
/// named on each.
pub trait StatesEngine {
    /// `Rand` (`zombie.core.random.Rand`): the game's random source.
    fn rand(&mut self) -> &mut dyn GameRand;

    /// Actual floor mutation service. State adapters already hold the cell
    /// and sprites and must not re-enter global WORLD to perform placement.
    fn add_world_inventory_item(
        &mut self,
        _cell: &mut IsoCell,
        _meta: &IsoMetaGrid,
        _sprites: &mut IsoSpriteManager,
        _square: (i32, i32, i32),
        _item: &pz_inventory::shared::ItemRef,
        _x: f32,
        _y: f32,
        _height: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        Err(pz_inventory::floor_drop::RuntimeHostError::Unsupported {
            operation: "actual state floor placement service",
            kind: None,
        })
    }

    fn add_world_inventory_item_on(
        &mut self,
        _cell: &mut IsoCell,
        _meta: &IsoMetaGrid,
        _sprites: &mut IsoSpriteManager,
        _square: &pz_world::SquareRef,
        _item: &pz_inventory::shared::ItemRef,
        _x: f32,
        _y: f32,
        _height: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        Err(pz_inventory::floor_drop::RuntimeHostError::Unsupported {
            operation: "actual retained state floor placement service",
            kind: None,
        })
    }

    fn validate_floor_session(&self) -> pz_inventory::floor_drop::HostResult<()> {
        Err(pz_inventory::floor_drop::RuntimeHostError::Unsupported {
            operation: "actual state floor session",
            kind: None,
        })
    }

    fn floor_fbo_render_chunk(&self) -> pz_inventory::floor_drop::HostResult<bool> {
        Err(pz_inventory::floor_drop::RuntimeHostError::Unsupported {
            operation: "actual floor render configuration",
            kind: None,
        })
    }

    // ---- the live character animation owner ----
    fn thumps_this_frame(&mut self, _chr: i32) -> i32 {
        0
    }
    fn turn_alerted_enter(&mut self, _chr: i32, _target_angle: f32) {}
    fn deferred_movement_step(
        &mut self,
        _chr: i32,
        _input: &pz_anim::DeferredMovementInput,
    ) -> Option<(f32, f32)> {
        None
    }
    fn deferred_movement_length_sq(&self, _chr: i32) -> f32 {
        0.0
    }
    fn contains_any_idle_nodes(&self, _chr: i32) -> bool {
        false
    }
    fn animation_player_ready(&self, _chr: i32) -> bool {
        false
    }
    fn rendered_angle(&self, _chr: i32) -> Option<f32> {
        None
    }
    fn primary_hand_model_ready(&self, _chr: i32) -> bool {
        false
    }

    // ---- physics, firearms and ragdolls ----

    /// `WorldSimulation`'s shared Bullet physics world (`StateWorld::physics`).
    fn physics(&mut self) -> Option<&mut dyn pz_characters::character::firearm::PhysicsWorld> {
        None
    }
    /// Primary hand model's `MUZZLE` attachment world position/direction.
    fn muzzle_attachment(
        &self,
        _chr: i32,
    ) -> Option<pz_characters::character::firearm::MuzzleAttachment> {
        None
    }
    /// Aiming reticle in isometric world coordinates.
    fn aim_reticle_iso(&self, _chr: i32) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// Character's animation skeleton packed for `BallisticsTarget`.
    fn ballistics_skeleton(
        &self,
        _chr: i32,
    ) -> Option<pz_characters::character::firearm::TargetSkeleton> {
        None
    }
    /// Vehicle aim modifiers for a character inside a vehicle.
    fn vehicle_aim(&self, _chr: i32) -> Option<pz_characters::character::firearm::VehicleAim> {
        None
    }
    /// Live `RagdollController` pelvis world position, if ragdolling.
    fn ragdoll_pelvis(&self, _chr: i32) -> Option<[f32; 3]> {
        None
    }
    /// Animal definitions and [`pz_animals::AnimalWorld`] bound to `chr`
    /// (`StateWorld::animal_host`).
    fn animal_host(&mut self, _chr: i32) -> Option<pz_characters::character::animal::AnimalHost<'_>> {
        None
    }
    /// Called by `StateWorld::virtualize_animal_if_unloaded` when `chr` walks
    /// toward an unloaded square outside a building. Returns `true` if the
    /// engine accepted the virtualization request.
    fn on_virtualize_animal(&mut self, _chr: i32) -> bool {
        false
    }

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
    /// Active close-window actions, captured before the world borrows its actors.
    fn closing_windows(&self) -> &[ClosingWindow] { &[] }
    /// The zombies among `square.getMovingObjects()` (the squares'
    /// moving-object lists are the character owner's), in list order.
    fn zombies_on_square(&self, _sq: (i32, i32, i32)) -> Vec<i32> {
        Vec::new()
    }
    /// `WorldDictionary.getObjectNameFromID(id)` for an object name stored
    /// as a dictionary id.
    fn dict_object_name(&self, _id: i8) -> Option<String> {
        None
    }
    /// The ids of `square.getMovingObjects()` (characters), in list order.
    fn moving_objects_on_square(&self, _sq: (i32, i32, i32)) -> Vec<i32> {
        Vec::new()
    }
    /// `BentFences.getInstance()` with the Lua-registered tiles; `None`:
    /// no bendable fences.
    fn bent_fences(&self) -> Option<std::rc::Rc<BentFences>> {
        None
    }
    /// `BrokenFences.getInstance().addItems(obj, square)` (the debris items
    /// of a removed fence piece; draws `Rand`).
    fn broken_fence_items(&mut self, _sq: (i32, i32, i32), _obj: &IsoObjectRef) {}
    /// `BentFences.collapse`'s characters: the zombies in `trip` that are
    /// not behind `fence` fall on their front (`setOnFloor`,
    /// `setFallOnFront`, hit reaction `Floor`); in `crush`, zombies fall
    /// (on their front when behind it) and players bump and fall
    /// (`pushedBehind` / `pushedFront`).
    fn fence_collapsed(&mut self, _fence: ObjectRef, _trip: &[i32], _crush: &[i32]) {}

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

    fn geometry_changed(&mut self, _cell: &IsoCell, _meta: &IsoMetaGrid,
        _sprites: &IsoSpriteManager, pos: (i32, i32, i32)) {
        self.square_changed(pos);
    }

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
    fn dump_contents(&mut self, _sq: (i32, i32, i32), _obj: &IsoObjectRef) {}
    /// `obj` left `sq` (`OnObjectAboutToBeRemoved` / `OnTileRemoved`, and
    /// `OnDestroyIsoThumpable` for destroyed thumpables when `destroyed`).
    fn object_removed(&mut self, _sq: (i32, i32, i32), _obj: &IsoObjectRef, _destroyed: bool) {}
    /// `LuaEventManager.triggerEvent("OnContainerUpdate")`.
    fn on_container_update(&mut self) {}
    fn on_window_toggled(&mut self, _actor: i32, _window: &IsoObjectRef) {
        self.on_container_update();
    }
    /// `building.forceAwake()`.
    fn force_awake(&mut self, _building: BuildingIdx) {}
    /// `AmbientStreamManager.instance.doAlarm(def)` for an alarmed building.
    fn do_alarm(&mut self, _building: BuildingIdx) {}
    fn do_room_alarm(&mut self, _room: i64, building: BuildingIdx) { self.do_alarm(building) }
    fn zombie_triggers_house_alarm(&self) -> bool { false }
    /// `IsoDoor/IsoThumpable.ToggleDoor(chr)` on the cell (the door code
    /// with its sounds, Lua and path invalidation).
    fn toggle_door(&mut self, _cell: &mut IsoCell, _door: ObjectRef, _chr: i32) {}
    /// `IsoWindow.ToggleWindow(chr)` on the cell.
    fn toggle_window(&mut self, cell: &mut IsoCell, meta: &IsoMetaGrid, sprites: &IsoSpriteManager,
        window: ObjectRef, actor: &Thumper) -> pz_inventory::floor_drop::HostResult<()> {
        let Some(square) = cell.get_grid_square(window.x, window.y, window.z) else { return Ok(()) };
        let object = {
            let square = square.try_borrow().map_err(|_| pz_inventory::floor_drop::RuntimeHostError::Borrowed("window square"))?;
            square.object_index(window.id).map(|index| square.objects[index].ref_handle())
        };
        if let Some(object) = object {
            if pz_world::world_state::toggle_window(cell, meta, sprites, (window.x, window.y, window.z),
                &object, Some((actor.x, actor.y)))? {
                if !actor.zombie || self.zombie_triggers_house_alarm() {
                    if let Some((room, building)) = pz_world::world_state::opened_window_alarm(cell, meta, (window.x, window.y, window.z), &object)? {
                        self.do_room_alarm(room, building);
                    }
                }
                self.geometry_changed(cell, meta, sprites, (window.x, window.y, window.z));
                self.on_window_toggled(actor.id, &object);
            }
        }
        Ok(())
    }
    /// `LuaEventManager.triggerEvent("OnWeaponHitThumpable", chr, null,
    /// obj)` (a bare-handed window smash).
    fn weapon_hit_thumpable(&mut self, _chr: i32, _obj: ObjectRef) {}
    /// `VehicleWindow.isDestroyed()` of a vehicle's part (the vehicles'
    /// owner; `None`: no such window).
    fn vehicle_window_destroyed(&self, id: i16, part: &str) -> Option<bool> {
        let part = pz_vehicles::manager::vehicle_by_id(id)?.part_by_id(part)?;
        let state = part.st();
        state.window.map(|_| state.condition == 0)
    }

    fn vehicle_window_hit(&mut self, id: i16, part: &str, _actor: i32) {
        if let Some(part) = pz_vehicles::manager::vehicle_by_id(id).and_then(|v| v.part_by_id(part)) {
            if part.st().window.is_some() { part.hit_window() }
        }
    }

    /// The local player's input this frame (`pressedMovement`,
    /// `pressedCancelAction` in the window states).
    fn player_input(&self) -> Option<&dyn pz_characters::character::player::PlayerInput> {
        None
    }
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
    /// `Core.getOptionDoDoorSpriteEffects()`.
    fn door_sprite_effects_enabled(&self) -> bool { true }

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

/// The host's key of `chr`: its `IsoMovingObject` id, the low half of its
/// mover id ([`GameCharacter::mover_id_of`]).
fn character_id(chr: &dyn PathMover) -> i32 {
    chr.mover_id().0 as u32 as i32
}

/// [`StateWorld`] over the loaded cell.
pub struct StatesWorldAdapter<'a> {
    pub cell: &'a mut IsoCell,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a mut IsoSpriteManager,
    pub settings: ThumpSettings,
    pub runtime: &'a mut StatesRuntime,
    pub engine: &'a mut dyn StatesEngine,
    /// The shared gameplay sound list for character hearing.
    sounds: Option<&'a pz_audio::world_sound::WorldSoundManager>,
    /// The other characters the states reach by id (the owner's
    /// `Vec<GameCharacter<I>>`, set with [`Self::with_characters`]); `None`:
    /// no other characters.
    characters: Option<&'a mut dyn std::any::Any>,
    /// The list index of each character out with `take_character`.
    taken: Vec<(i32, usize)>,
    /// The world's path finding (`PathfindNative.instance` and each
    /// character's `PathFindBehavior2`), set with [`Self::with_path`];
    /// `None`: the path calls do nothing.
    path: Option<(&'a mut PathHost, &'a dyn PathWorldHooks)>,
}

/// The characters a `pathToCharacter` goal is looked up in: the list the
/// adapter reaches by id. The character that asks is out of it.
struct ListLookup<'l, I: 'static>(&'l [GameCharacter<I>]);

impl<I: 'static> CharacterLookup for ListLookup<'_, I> {
    fn character(&self, id: MoverId) -> Option<&dyn CharacterView> {
        self.0.iter().find(|c| c.mover_id_of() == id).map(|c| c as &dyn CharacterView)
    }
}

impl<'a> StatesWorldAdapter<'a> {
    pub fn with_world_sounds(mut self, sounds: &'a pz_audio::world_sound::WorldSoundManager) -> Self {
        self.sounds = Some(sounds);
        self
    }

    /// Lets the states reach `characters` by id (`character_mut`,
    /// `take_character`, `characters_near`).
    pub fn with_characters<I: 'static>(mut self, characters: &'a mut Vec<GameCharacter<I>>) -> Self {
        self.characters = Some(characters);
        self
    }

    /// Lets the states find paths through `host` (`PathFindBehavior2`'s
    /// calls), with `hooks` for the game settings they read.
    pub fn with_path(mut self, host: &'a mut PathHost, hooks: &'a dyn PathWorldHooks) -> Self {
        self.path = Some((host, hooks));
        self
    }

    /// `f` with the path host and the world as the behaviours see it: the
    /// squares, the game settings and the other characters.
    fn with_path_env<I: 'static, R>(
        &mut self,
        f: impl FnOnce(&mut PathHost, &PathEnv<'_, '_, '_, IsoCell>) -> R,
    ) -> Option<R> {
        let (host, hooks) = self.path.as_mut()?;
        let bits_hooks = pz_pathfind::square_bits::DefaultBitsHooks;
        let bits = pz_pathfind::square_bits::SquareBits::new(&*self.sprites, self.meta, &*self.cell, &bits_hooks).ok()?;
        let map = pz_pathfind::main_thread::MainThreadMap::new(&bits, &pz_pathfind::game_vehicle::NoVehicles);
        let list: &[GameCharacter<I>] = self
            .characters
            .as_deref()
            .and_then(|c| c.downcast_ref::<Vec<GameCharacter<I>>>())
            .map_or(&[], |l| l.as_slice());
        let env = PathEnv {
            map: &map,
            world: *hooks,
            vehicles: &pz_pathfind::behavior::NoVehicleScripts,
            seating: &pz_pathfind::behavior::NoSeating,
            characters: &ListLookup(list),
        };
        Some(f(host, &env))
    }

    fn character_list<I: 'static>(&mut self) -> Option<&mut Vec<GameCharacter<I>>> {
        self.characters.as_deref_mut()?.downcast_mut()
    }

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
            sounds: None,
            characters: None,
            taken: Vec::new(),
            path: None,
        }
    }

    /// The read-only queries over the cell.
    pub fn env(&self) -> Env<'_> {
        Env::new(self.cell, self.sprites, self.meta)
    }

    /// Retain an overwritten actual sprite until the outer host retirement
    /// boundary, after the object guard and any caller WORLD guard are gone.
    fn replace_object_sprite_tile(&mut self, object: &IsoObjectRef, sprite: Option<usize>) {
        let (old, native_class) = {
            let mut object = object.borrow_mut();
            let native_class = object.kind.native_class_name();
            let old = std::mem::replace(&mut object.sprite, sprite.into());
            (old, native_class)
        };
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_payload(Box::new(old));
        self.cell.retire_components(retired);
        pz_world::object_slot::observe_object(object, native_class);
    }

    fn at(&self, p: (i32, i32, i32)) -> Option<pz_world::SquareRef> {
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
        let (sq, id) = env.test_collide_special_objects(&this, env.at(to).as_ref())?;
        let (x, y, z) = {
            let sq = sq.borrow();
            (sq.x, sq.y, sq.z)
        };
        Some(ObjectRef { x, y, z, id })
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
        let step = |a: Option<&SquareRef>, b: Option<&SquareRef>, doors: bool| -> bool {
            let (Some(a), Some(b)) = (a, b) else {
                return false;
            };
            let (a, b) = (a.borrow(), b.borrow());
            let mut blocked =
                a.calculate_collide(Some(&b), false, false, true, true, self.sprites, &env);
            if doors && !ignore_doors && a.is_door_blocked_to(Some(&b), &env, &hooks) {
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
                if step(a.as_ref(), b.as_ref(), true) {
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
                if step(a.as_ref(), b.as_ref(), true) {
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
                if step(a.as_ref(), b.as_ref(), false) {
                    return true;
                }
                b = a;
            }
        }
        false
    }

    /// The corpse with `id` (see [`corpse_id`]).
    fn corpse(&self, id: i64) -> Option<(pz_world::SquareRef, usize)> {
        let (x, y, z, i) = corpse_pos(id);
        let sq_owner = self.cell.get_grid_square(x, y, z)?;
        let exists = i < sq_owner.borrow().static_moving.len();
        exists.then_some((sq_owner, i))
    }

    fn corpse_info(&self, sq: &IsoGridSquare, i: usize) -> CorpseInfo {
        let b = sq.static_moving[i].borrow();
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

// Hearing queries need the same loaded square and room a character occupies.
// Mutations register through StatesEngine after the state releases its cell.
impl pz_audio::world_sound::SoundWorld for StatesWorldAdapter<'_> {
    fn is_server(&self) -> bool { self.settings.server }
    fn is_client(&self) -> bool { self.settings.client }
    fn chunk_loaded(&self, wx: i32, wy: i32) -> bool { self.cell.chunks.contains_key(&(wx, wy)) }
    fn room_at(&self, x: i32, y: i32, z: i32) -> pz_audio::world_sound::RoomAt {
        pz_world_sound::sound_room_at(self.cell, x, y, z)
    }
}

impl<I: 'static> StateWorld<I> for StatesWorldAdapter<'_> {
    fn world_sounds(&self) -> Option<(&pz_audio::world_sound::WorldSoundManager, &dyn pz_audio::world_sound::SoundWorld)> {
        Some((self.sounds?, self))
    }

    fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<I>> {
        self.character_list::<I>()?.iter_mut().find(|c| c.id == id)
    }
    fn path_to_location(&mut self, chr: &mut dyn PathMover, x: i32, y: i32, z: i32) {
        if let Some((host, _)) = self.path.as_mut() {
            host.path_to_location(character_id(chr), chr, x, y, z);
        }
    }
    fn path_to_location_f(&mut self, chr: &mut dyn PathMover, x: f32, y: f32, z: f32) {
        if let Some((host, _)) = self.path.as_mut() {
            host.path_to_location_f(character_id(chr), chr, x, y, z);
        }
    }
    fn path_to_sound(&mut self, chr: &mut dyn PathMover, x: i32, y: i32, z: i32) {
        if let Some((host, _)) = self.path.as_mut() {
            host.path_to_sound(character_id(chr), chr, x, y, z);
        }
    }
    fn path_to_character(&mut self, chr: &mut dyn PathMover, target: MoverId) {
        self.with_path_env::<I, _>(|host, env| host.path_to_character(character_id(chr), chr, env, target));
    }
    fn path_update(&mut self, chr: &mut dyn PathMover) -> BehaviorResult {
        self.with_path_env::<I, _>(|host, env| host.path_update(character_id(chr), chr, env))
            .unwrap_or(BehaviorResult::Failed)
    }
    fn path_cancel(&mut self, chr: &mut dyn PathMover) {
        if let Some((host, _)) = self.path.as_mut() {
            host.path_cancel(character_id(chr), chr);
        }
    }
    fn path_reset(&mut self, chr: &mut dyn PathMover) {
        if let Some((host, _)) = self.path.as_mut() {
            host.path_reset(character_id(chr), chr);
        }
    }
    fn path_behavior(&self, id: i32) -> Option<&PathFindBehavior2> {
        self.path.as_ref()?.0.behavior(id)
    }
    fn walking_on_the_spot_reset(&mut self, id: i32, x: f32, y: f32) {
        if let Some((host, _)) = self.path.as_mut() {
            host.walking_on_the_spot_reset(id, x, y);
        }
    }
    fn walking_on_the_spot_check(&mut self, id: i32, x: f32, y: f32) -> bool {
        match self.path.as_mut() {
            Some((host, hooks)) => host.walking_on_the_spot_check(id, x, y, *hooks),
            None => false,
        }
    }
    /// The last character fills the taken one's place, so a take and its
    /// put are O(1) (the zombie update takes every zombie each frame).
    fn take_character(&mut self, id: i32) -> Option<GameCharacter<I>> {
        let list = self.character_list::<I>()?;
        let i = list.iter().position(|c| c.id == id)?;
        let chr = list.swap_remove(i);
        self.taken.push((id, i));
        Some(chr)
    }
    /// Puts the character back at its index and the one that filled it
    /// back at the end: puts in the reverse order of the takes restore the
    /// list order exactly.
    fn put_character(&mut self, chr: GameCharacter<I>) {
        let at = self.taken.iter().rposition(|t| t.0 == chr.id).map(|k| self.taken.remove(k).1);
        let Some(list) = self.character_list::<I>() else { return };
        list.push(chr);
        let last = list.len() - 1;
        if let Some(i) = at.filter(|i| *i < last) {
            list.swap(i, last);
        }
    }
    fn characters_near(&self, x: f32, y: f32, _z: f32, radius: f32) -> Vec<i32> {
        let Some(list) = self.characters.as_deref().and_then(|c| c.downcast_ref::<Vec<GameCharacter<I>>>()) else {
            return Vec::new();
        };
        list.iter()
            .filter(|c| {
                let (dx, dy) = (c.vars.x - x, c.vars.y - y);
                dx * dx + dy * dy <= radius * radius
            })
            .map(|c| c.id)
            .collect()
    }
    fn floor(&mut self) -> Option<&mut dyn pz_inventory::floor_drop::FloorWorld> {
        Some(self)
    }

    fn thumps_this_frame(&mut self, chr: i32) -> i32 {
        self.engine.thumps_this_frame(chr)
    }
    fn turn_alerted_enter(&mut self, chr: i32, target_angle: f32) {
        self.engine.turn_alerted_enter(chr, target_angle);
    }
    fn deferred_movement_step(
        &mut self,
        chr: i32,
        input: &pz_anim::DeferredMovementInput,
    ) -> Option<(f32, f32)> {
        self.engine.deferred_movement_step(chr, input)
    }
    fn deferred_movement_length_sq(&self, chr: i32) -> f32 {
        self.engine.deferred_movement_length_sq(chr)
    }
    fn contains_any_idle_nodes(&self, chr: i32) -> bool {
        self.engine.contains_any_idle_nodes(chr)
    }
    fn animation_player_ready(&self, chr: i32) -> bool {
        self.engine.animation_player_ready(chr)
    }
    fn rendered_angle(&self, chr: i32) -> Option<f32> {
        self.engine.rendered_angle(chr)
    }
    fn primary_hand_model_ready(&self, chr: i32) -> bool {
        self.engine.primary_hand_model_ready(chr)
    }
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        let sq_owner = self.cell.get_grid_square(x, y, z)?;
        let sq = sq_owner.borrow();
        Some(self.square_info(&sq))
    }

    fn context_sheet_rope_side(&self, q: &ContextQuery, dir: IsoDirections) -> bool {
        self.context_sheet_rope_side_at(q, dir)
    }

    fn context_door_or_window_object(&self, q: &ContextQuery, dir: IsoDirections) -> Option<ObjectRef> {
        self.context_door_or_window_object_at(q, dir)
    }

    fn context_do_door_or_window(
        &mut self,
        q: &ContextQuery,
        dir: IsoDirections,
        obj: ObjectRef,
    ) -> pz_inventory::floor_drop::HostResult<Vec<ContextualAction>> {
        self.context_do_door_or_window_at(q, dir, obj)
    }

    fn context_special(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        which: ContextSpecial,
    ) -> pz_inventory::floor_drop::HostResult<Option<ContextualAction>> {
        self.context_special_at(q, dir, which)
    }

    fn context_hoppable(&self, q: &ContextQuery, dir: IsoDirections) -> Option<((i32, i32, i32), Option<i64>)> {
        self.context_hoppable_at(q, dir)
    }

    fn context_safe_to_climb_over(&self, q: &ContextQuery, dir: IsoDirections) -> bool {
        self.is_safe_to_climb_over(q.square, dir)
    }

    fn context_toggle_door(
        &mut self,
        chr: i32,
        action: &ContextualAction,
        actor: &pz_characters::character::player::context::DoorActor<'_>,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        let (Some((x, y, z)), Some(id)) = (action.square, action.object) else { return Ok(()) };
        self.toggle_player_door(ObjectRef { x, y, z, id: id as u32 }, chr, actor)
    }

    fn characters_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
        self.at(sq).map_or_else(Vec::new, |s| s.borrow().moving_objects.clone())
    }
    fn remove_moving_object(&mut self, sq: (i32, i32, i32), chr: i32) {
        if let Some(s) = self.at(sq) {
            let mut s = s.borrow_mut();
            if let Some(i) = s.moving_objects.iter().position(|&id| id == chr) {
                s.moving_objects.remove(i);
            }
        }
    }
    fn add_moving_object(&mut self, sq: (i32, i32, i32), chr: i32) {
        if let Some(s) = self.at(sq) {
            let mut s = s.borrow_mut();
            if !s.moving_objects.contains(&chr) {
                s.moving_objects.push(chr);
            }
        }
    }

    fn sloped_surface_height(&self, sq: (i32, i32, i32), dx: f32, dy: f32) -> f32 {
        self.at(sq)
            .map_or(0.0, |s| s.borrow().get_sloped_surface_height_at(dx, dy))
    }

    fn wall_type(&self, sq: (i32, i32, i32)) -> i32 {
        self.at(sq).map_or(0, |s| self.wall_type_of(&s.borrow()))
    }

    fn vault_over(
        &self,
        current: (i32, i32, i32),
        at: (i32, i32, i32),
        dir: IsoDirections,
        collided: (bool, bool, bool, bool),
    ) -> Option<(IsoDirections, pz_characters::character::state::FenceClimb)> {
        self.vault_over_for(current, at, dir, collided)
    }

    fn window_climb(
        &self,
        chr: &dyn CharacterView,
        kind: pz_world::square::MoverKind,
        window: ObjectRef,
    ) -> Option<pz_characters::character::state::WindowClimb> {
        let t = Thumper {
            x: chr.x(),
            y: chr.y(),
            z: chr.z(),
            current_square: chr.current_square(),
            zombie: kind == pz_world::square::MoverKind::Zombie,
            animal: kind == pz_world::square::MoverKind::Animal,
            ..Default::default()
        };
        self.window_climb_for(&t, window)
    }

    fn window_between(
        &self,
        start: (i32, i32, i32),
        end: (i32, i32, i32),
    ) -> Option<(ObjectRef, pz_characters::character::state::ThumpKind)> {
        self.window_between_squares(start, end)
    }

    fn window_closing(&self, window: ObjectRef) -> bool {
        if self.engine.closing_windows().is_empty() { return false }
        let Some((square, object)) = self.find(window) else { return false };
        let object = object.borrow();
        let pz_world::objects::ObjectKind::Window(w) = &object.kind else { return false };
        let square = square.borrow();
        self.first_window_closer_succeeds(&square, window, w.north)
    }

    fn reroute_collide(&self, obj: ObjectRef) -> bool {
        self.find(obj)
            .is_some_and(|(_, o)| o.borrow().reroute_collide.is_some())
    }

    fn has_broken_glass(&self, sq: (i32, i32, i32)) -> bool {
        self.broken_glass_at(sq)
    }

    fn zombies_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
        if self.at(sq).is_none() {
            return Vec::new();
        }
        self.engine.zombies_on_square(sq)
    }

    fn damage_thumpable(&mut self, obj: ObjectRef, damage: i32) {
        self.damage_thumpable_at(obj, damage);
    }

    fn hoppable_between(&self, start: (i32, i32, i32), end: (i32, i32, i32)) -> Option<ObjectRef> {
        self.hoppable_between_squares(start, end)
    }

    fn fence_info(&self, fence: ObjectRef) -> Option<pz_characters::character::state::FenceInfo> {
        self.fence_info_of(fence)
    }

    fn damage_fence(&mut self, fence: ObjectRef, damage: i32, dir: IsoDirections) {
        self.damage_fence_at(fence, damage, dir);
    }

    fn square_room(&self, sq: (i32, i32, i32)) -> Option<i64> {
        self.at(sq).and_then(|s| {
            let id = s.borrow().room_id;
            (id != -1).then_some(id)
        })
    }

    fn square_building(&self, sq: (i32, i32, i32)) -> Option<i64> {
        let s = self.at(sq)?;
        let building = self.square_building_idx(&s.borrow());
        building.map(|b| b.0 as i64)
    }

    fn square_smoke(&self, sq: (i32, i32, i32)) -> bool {
        self.at(sq)
            .is_some_and(|s| self.env().sq_has(&s.borrow(), "smoke"))
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
                let s = s.borrow();
                s.objects
                    .iter()
                    .filter_map(|o| env.sprite_name(&o.borrow()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn grid_sneak_modifier(&self, sq: (i32, i32, i32), only_solid_trans: bool) -> f32 {
        self.at(sq).map_or(1.0, |s| {
            self.env()
                .grid_sneak_modifier(&s.borrow(), only_solid_trans)
        })
    }

    fn is_something_to(&self, a: (i32, i32, i32), b: (i32, i32, i32)) -> bool {
        match (self.at(a), self.at(b)) {
            (Some(a), Some(b)) => self.env().is_something_to(&a.borrow(), &b.borrow()),
            _ => false,
        }
    }

    /// `isFree(true)`.
    fn square_is_free(&self, sq: (i32, i32, i32)) -> bool {
        let Some(s) = self.at(sq) else { return false };
        !self.engine.has_moving_objects(sq) && self.env().is_free(&s.borrow())
    }

    fn same_staircase(&self, sq: (i32, i32, i32), x: i32, y: i32, z: i32) -> bool {
        self.at(sq)
            .is_some_and(|s| self.env().is_same_staircase(&s.borrow(), x, y, z))
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
            let s = s.borrow();
            let env = self.env();
            ["climbSheetN", "climbSheetE", "climbSheetS", "climbSheetW"]
                .into_iter()
                .any(|n| env.sq_has(&s, n))
                && env.sheet_rope_index(&s).is_some()
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
        let Some(sq_owner) = self.cell.get_grid_square(from.0, from.1, from.2) else {
            return pz_world::square::CollideAdjacent {
                blocked: true,
                ..Default::default()
            };
        };
        self.collide_env()
            .test_collide_adjacent(&sq_owner, Some(mover), dx, dy, dz)
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
        let climb = self.fence_climb_for(&t, from, dir)?;
        if !zombie {
            return Some(climb);
        }
        // BentFences.checkDamageHoppableFence, then setParams reads the
        // square beyond.
        let (dx, dy) = dir.delta();
        let opp = (from.0 + dx, from.1 + dy, from.2);
        self.check_damage_hoppable_fence(from, opp);
        Some(self.fence_opposite(self.at(opp).as_ref().map(|s| s.borrow()).as_deref()))
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
                let s = s.borrow();
                s.has_type(self.sprites.tables.ty.tree)
                    || s.objects.iter().any(|o| {
                        env.metadata(&o.borrow()).is_some_and(|spr| {
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
            mutation: f.mutation,
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

    fn floor_sprite_name(&self, sq: (i32, i32, i32)) -> Option<String> {
        self.floor_sprite_name_at(sq)
    }

    fn path_cancel_request(&mut self, chr: &mut dyn pz_pathfind::mover::PathMover) {
        match self.path.as_mut() {
            Some((host, _)) => host.service_mut().cancel_request(chr.mover_id()),
            None => self.engine.path_cancel_request(chr.mover_id()),
        }
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
        let s = s.borrow();
        (!s.static_moving.is_empty()).then(|| self.corpse_info(&s, 0))
    }

    fn dead_body(&self, id: i64) -> Option<CorpseInfo> {
        let (sq, i) = self.corpse(id)?;
        let info = self.corpse_info(&sq.borrow(), i);
        Some(info)
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

    fn square_is_outside(&self, sq: (i32, i32, i32)) -> bool {
        self.at(sq)
            .is_some_and(|s| self.env().sq_has(&s.borrow(), "exterior"))
    }

    fn physics(&mut self) -> Option<&mut dyn pz_characters::character::firearm::PhysicsWorld> {
        self.engine.physics()
    }

    fn animal_host(
        &mut self,
        chr: i32,
    ) -> Option<pz_characters::character::animal::AnimalHost<'_>> {
        self.engine.animal_host(chr)
    }

    fn virtualize_animal_if_unloaded(&mut self, chr: i32, next_x: i32, next_y: i32) -> bool {
        if self.cell.get_grid_square(next_x, next_y, 0).is_some() {
            return false;
        }
        if self.meta.get_associated_building_at(next_x, next_y).ok().flatten().is_some() {
            return false;
        }
        self.engine.on_virtualize_animal(chr)
    }

    fn muzzle_attachment(
        &self,
        chr: i32,
    ) -> Option<pz_characters::character::firearm::MuzzleAttachment> {
        self.engine.muzzle_attachment(chr)
    }

    fn aim_reticle_iso(&self, chr: i32) -> (f32, f32) {
        self.engine.aim_reticle_iso(chr)
    }

    fn ballistics_skeleton(
        &self,
        chr: i32,
    ) -> Option<pz_characters::character::firearm::TargetSkeleton> {
        self.engine.ballistics_skeleton(chr)
    }

    fn first_blocking_square(
        &self,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> pz_characters::character::firearm::BlockingSquare {
        use pz_characters::character::state::TestResults as CharTestResults;
        let hooks = self.env().hooks();
        let env = VisionEnv::new(&*self.cell, self.sprites, &hooks);
        let (mut x0, mut y0, mut z0) = from;
        let (x1, y1, z1) = to;
        let mut prev = self.at((x0, y0, z0));
        let mut dy = y1 - y0;
        let mut dx = x1 - x0;
        let mut dz = z1 - z0;
        let mut t = 0.5f32;
        let mut t2 = 0.5f32;
        let mut best = pz_world::los::TestResults::Clear;

        let mut step = |nx: i32, ny: i32, nz: i32| -> Option<pz_characters::character::firearm::BlockingSquare> {
            let next = self.at((nx, ny, nz));
            if let (Some(a), Some(b)) = (&next, &prev) {
                let (ab, bb) = (a.borrow(), b.borrow());
                let res = ab.test_vision_adjacent(
                    bb.x - ab.x,
                    bb.y - ab.y,
                    bb.z - ab.z,
                    true,
                    false,
                    &env,
                );
                if res == pz_world::los::TestResults::Blocked {
                    return Some(pz_characters::character::firearm::BlockingSquare {
                        result: CharTestResults::Blocked,
                        square: Some((nx, ny, nz)),
                        position: (nx as f32 + 0.5, ny as f32 + 0.5),
                    });
                }
                if best == pz_world::los::TestResults::Clear {
                    best = res;
                }
            }
            prev = next;
            None
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
                if let Some(hit) = step(x0, t.floor() as i32, t2.floor() as i32) {
                    return hit;
                }
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
                t += m;
                y0 += dy;
                t2 += m2;
                if let Some(hit) = step(t.floor() as i32, y0, t2.floor() as i32) {
                    return hit;
                }
            }
        } else if dz != 0 {
            let mut m = dx as f32 / dz as f32;
            let mut m2 = dy as f32 / dz as f32;
            t += x0 as f32;
            t2 += y0 as f32;
            dz = if dz < 0 { -1 } else { 1 };
            m *= dz as f32;
            m2 *= dz as f32;
            while z0 != z1 {
                t += m;
                t2 += m2;
                z0 += dz;
                if let Some(hit) = step(t.floor() as i32, t2.floor() as i32, z0) {
                    return hit;
                }
            }
        }

        let mapped = match best {
            pz_world::los::TestResults::Clear => CharTestResults::Clear,
            pz_world::los::TestResults::ClearThroughOpenDoor => {
                CharTestResults::ClearThroughOpenDoor
            }
            pz_world::los::TestResults::ClearThroughWindow => CharTestResults::ClearThroughWindow,
            pz_world::los::TestResults::Blocked => CharTestResults::Blocked,
            pz_world::los::TestResults::ClearThroughClosedDoor => {
                CharTestResults::ClearThroughClosedDoor
            }
        };
        pz_characters::character::firearm::BlockingSquare {
            result: mapped,
            square: None,
            position: (x1 as f32 + 0.5, y1 as f32 + 0.5),
        }
    }

    fn vehicle_aim(&self, chr: i32) -> Option<pz_characters::character::firearm::VehicleAim> {
        self.engine.vehicle_aim(chr)
    }

    fn ragdoll_pelvis(&self, chr: i32) -> Option<[f32; 3]> {
        self.engine.ragdoll_pelvis(chr)
    }

    // ---- windows, sheet ropes and walls ----

    fn player_input(&self) -> Option<&dyn pz_characters::character::player::PlayerInput> {
        self.engine.player_input()
    }

    fn window_info(&self, window: ObjectRef) -> Option<WindowInfo> {
        self.window_info_of(window)
    }

    fn toggle_window(&mut self, window: ObjectRef, chr: &GameCharacter<I>) -> pz_inventory::floor_drop::HostResult<()> {
        self.engine.toggle_window(self.cell, self.meta, self.sprites, window, &Thumper::of(chr))
    }

    fn debug_unlock_window(&mut self, window: ObjectRef, opening: bool) -> pz_inventory::floor_drop::HostResult<()> {
        self.debug_unlock_window_at(window, opening)
    }
    fn set_window_perma_locked(&mut self, window: ObjectRef, perma: bool) {
        self.set_window_perma_locked_at(window, perma);
    }

    fn window_hit_target(&self, window: ObjectRef, chr: &GameCharacter<I>) -> Option<pz_characters::character::state::cx::WindowHitTarget> {
        let location = self.thumpable_for(window, Some(&Thumper::of(chr)))?;
        let object = self.object_ref(location)?;
        if matches!(&object.borrow().kind, pz_world::objects::ObjectKind::Barricade(b) if b.is_destroyed()) {
            return None;
        }
        Some(pz_characters::character::state::cx::WindowHitTarget { location, object })
    }

    fn window_weapon_hit(&mut self, window: ObjectRef, chr: &GameCharacter<I>) {
        let t = Thumper::of(chr);
        self.window_weapon_hit_by(window, &t, chr.is_player());
    }

    fn vehicle_window_destroyed(&self, vehicle: i16, part: &str) -> Option<bool> {
        self.engine.vehicle_window_destroyed(vehicle, part)
    }

    fn vehicle_window_hit(&mut self, vehicle: i16, part: &str, chr: &GameCharacter<I>) {
        self.engine.vehicle_window_hit(vehicle, part, chr.id);
    }

    fn sheet_rope_square(&self, sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
        self.sheet_rope_square_at(sq)
    }

    fn chunk_levels(&self, sq: (i32, i32, i32)) -> (i32, i32) {
        self.chunk_levels_at(sq)
    }

    fn sheet_rope_exit(
        &mut self,
        chr: &GameCharacter<I>,
        target: (i32, i32, i32),
        dir: IsoDirections,
    ) -> SheetRopeExit {
        let t = Thumper::of(chr);
        self.sheet_rope_exit_for(&t, target, dir)
    }

    fn can_climb_over_wall(&self, chr: &GameCharacter<I>, dir: IsoDirections) -> bool {
        self.can_climb_over_wall_for(&Thumper::of(chr), dir)
    }

    fn climb_wall_fence_type(
        &self,
        start: (i32, i32, i32),
        end: (i32, i32),
        dir: IsoDirections,
    ) -> Option<i32> {
        self.climb_wall_fence_type_at(start, end, dir)
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
