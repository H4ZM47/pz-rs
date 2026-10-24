//! [`PathHost`]: the path finding a game world hosts for its characters.
//!
//! In Java every `IsoGameCharacter` owns a `PathFindBehavior2` (`pfb2`,
//! created with the character) and `PathfindNative.instance` is a
//! singleton. Here the world owns one [`PathHost`], which holds the
//! [`PathfindService`] and one [`PathFindBehavior2`] per character id
//! (created on first use), routes the results of
//! [`PathfindService::update_main`] to the behaviour that asked by its
//! [`FinderId`], and drives the worker with [`PathHost::update_thread`].
//!
//! The methods have the shapes the character states call the world with
//! (`pz-characters`' `StateWorld`): a character id, the character as a
//! `&mut dyn PathMover` when the behaviour touches it, and the world
//! context the call reads ([`PathEnv`], [`MainThreadMap`],
//! [`PathWorldHooks`]) as arguments, since the host holds no borrow of the
//! world. The collision queries are associated functions over a
//! [`MainThreadMap`].
//!
//! The character id is the game's `IsoMovingObject` id (an `i32`); the
//! host does not interpret a [`MoverId`] beyond remembering each
//! character's for cancelling its request in [`PathHost::remove`].
//!
//! # Delivering results
//!
//! `PathfindNative.updateMain()` (once per frame, before `IsoWorld.update`)
//! calls `Succeeded` / `Failed` on each finished request's
//! `PathFindBehavior2`, which writes to its character (`setPath2`, the
//! finder progress). [`PathHost::update_main`] does the same through a
//! [`MoverAccess`] that lends the characters out by id, in the order the
//! worker finished the requests. A result for a character the world cannot
//! lend at that moment stays queued on its behaviour and is delivered at the
//! start of the next host call that gets that character
//! ([`PathHost::deliver`] does it explicitly); a removed character's
//! results are dropped.
//!
//! # A `StateWorld` delegating to it
//!
//! A sketch with a cut-down `StateWorld` (the real one is in
//! `pz-characters`; `chr_id` stands for however the world maps the mover
//! back to its character id):
//!
//! ```
//! use pz_pathfind::behavior::{
//!     BehaviorResult, CharacterLookup, NoSeating, NoVehicleScripts, PathFindBehavior2,
//! };
//! use pz_pathfind::game_vehicle::NoVehicles;
//! use pz_pathfind::host::{PathEnv, PathHost};
//! use pz_pathfind::main_thread::MainThreadMap;
//! use pz_pathfind::mover::{MoverId, PathMover, PathWorldHooks};
//! use pz_pathfind::square_bits::{DefaultBitsHooks, SquareBits};
//! use pz_world::{IsoCell, IsoMetaGrid, IsoSpriteManager};
//!
//! trait StateWorld {
//!     fn path_to_location(&mut self, chr: &mut dyn PathMover, x: i32, y: i32, z: i32);
//!     fn path_to_character(&mut self, chr: &mut dyn PathMover, target: i32);
//!     fn path_update(&mut self, chr: &mut dyn PathMover) -> BehaviorResult;
//!     fn path_cancel(&mut self, chr: &mut dyn PathMover);
//!     fn path_behavior(&self, id: i32) -> Option<&PathFindBehavior2>;
//!     fn walking_on_the_spot_check(&mut self, id: i32, x: f32, y: f32) -> bool;
//!     fn line_clear_collide(
//!         &self, x0: f32, y0: f32, x1: f32, y1: f32, z: i32, ignore: Option<i32>, flags: i32,
//!     ) -> bool;
//! }
//!
//! struct World {
//!     path: PathHost,
//!     cell: IsoCell,
//!     sprites: IsoSpriteManager,
//!     meta: IsoMetaGrid,
//!     game: Box<dyn PathWorldHooks>,
//!     others: Box<dyn CharacterLookup>,
//! }
//!
//! fn chr_id(chr: &dyn PathMover) -> i32 {
//!     chr.mover_id().0 as u32 as i32
//! }
//!
//! impl World {
//!     /// Runs `f` with the world as the behaviours see it.
//!     fn with_env<R>(&mut self, f: impl FnOnce(&mut PathHost, &PathEnv<'_, '_, '_, IsoCell>) -> R) -> R {
//!         let bits = SquareBits::new(&self.sprites, &self.meta, &self.cell, &DefaultBitsHooks).unwrap();
//!         let map = MainThreadMap::new(&bits, &NoVehicles);
//!         let env = PathEnv {
//!             map: &map,
//!             world: &*self.game,
//!             vehicles: &NoVehicleScripts,
//!             seating: &NoSeating,
//!             characters: &*self.others,
//!         };
//!         f(&mut self.path, &env)
//!     }
//!     /// The vehicle of character `id` (`IsoGameCharacter.getVehicle()`).
//!     fn vehicle_of(&self, _id: i32) -> Option<i16> {
//!         None
//!     }
//! }
//!
//! impl StateWorld for World {
//!     fn path_to_location(&mut self, chr: &mut dyn PathMover, x: i32, y: i32, z: i32) {
//!         self.path.path_to_location(chr_id(chr), chr, x, y, z);
//!     }
//!     fn path_to_character(&mut self, chr: &mut dyn PathMover, target: i32) {
//!         // The world knows the target's kind, hence its MoverId.
//!         let target = MoverId((3u64 << 32) | target as u32 as u64);
//!         self.with_env(|host, env| host.path_to_character(chr_id(chr), chr, env, target));
//!     }
//!     fn path_update(&mut self, chr: &mut dyn PathMover) -> BehaviorResult {
//!         self.with_env(|host, env| host.path_update(chr_id(chr), chr, env))
//!     }
//!     fn path_cancel(&mut self, chr: &mut dyn PathMover) {
//!         self.path.path_cancel(chr_id(chr), chr);
//!     }
//!     fn path_behavior(&self, id: i32) -> Option<&PathFindBehavior2> {
//!         self.path.behavior(id)
//!     }
//!     fn walking_on_the_spot_check(&mut self, id: i32, x: f32, y: f32) -> bool {
//!         self.path.walking_on_the_spot_check(id, x, y, &*self.game)
//!     }
//!     fn line_clear_collide(
//!         &self, x0: f32, y0: f32, x1: f32, y1: f32, z: i32, ignore: Option<i32>, flags: i32,
//!     ) -> bool {
//!         let bits = SquareBits::new(&self.sprites, &self.meta, &self.cell, &DefaultBitsHooks).unwrap();
//!         let map = MainThreadMap::new(&bits, &NoVehicles);
//!         let ignore_vehicle = ignore.and_then(|id| self.vehicle_of(id));
//!         PathHost::line_clear_collide(&map, x0, y0, x1, y1, z, ignore_vehicle, flags)
//!     }
//! }
//! ```

use std::collections::{HashMap, VecDeque};

use pz_world::GetSquare;

use crate::behavior::{
    BehaviorResult, CharacterLookup, Delivered, NoSeating, PathContext, PathFindBehavior2,
    SeatingHooks, VehicleScripts,
};
use crate::game_vehicle::VehicleSource;
use crate::main_thread::line_clear_collide::{FLAG_CLOSE_TO_WALLS, FLAG_IGNORE_DOORS};
use crate::main_thread::{
    fastfloor, line_clear, los_line_clear_collide, LineClearCollideMain, MainThreadMap, TestResults,
};
use crate::mover::{MoverId, PathMover, PathWorldHooks};
use crate::path::Path;
use crate::service::{FinderId, PathfindService, ThreadPass};

/// A character id (`IsoMovingObject.getID()`).
pub type CharacterId = i32;

/// What the behaviours read from the world on a call, besides the
/// pathfinder the host already holds: [`PathContext`] without the service.
pub struct PathEnv<'c, 'w, 'a, G: GetSquare> {
    /// Squares and vehicles.
    pub map: &'c MainThreadMap<'w, 'a, G>,
    pub world: &'c dyn PathWorldHooks,
    pub vehicles: &'c dyn VehicleScripts,
    pub seating: &'c dyn SeatingHooks,
    /// The other characters (a `pathToCharacter` goal), never the one the
    /// call is for.
    pub characters: &'c dyn CharacterLookup,
}

/// Lends the world's characters to [`PathHost::update_main`].
pub trait MoverAccess {
    /// Calls `f` with character `id`, if the world can lend it out now.
    fn with_mover(&mut self, id: CharacterId, f: &mut dyn FnMut(&mut dyn PathMover));
}

/// A character's `pfb2`, the mover id it last asked under and the results
/// not yet handed to it.
#[derive(Debug)]
struct Slot {
    behavior: PathFindBehavior2,
    mover: Option<MoverId>,
    pending: VecDeque<Option<Path>>,
}

impl Slot {
    /// `IPathfinder.Succeeded` / `Failed` for every queued result.
    fn flush(&mut self, chr: &mut dyn PathMover, seating: &dyn SeatingHooks) {
        self.mover = Some(chr.mover_id());
        while let Some(r) = self.pending.pop_front() {
            match r {
                Some(path) => self.behavior.succeeded(chr, seating, &path),
                None => self.behavior.failed(chr),
            }
        }
    }
}

/// The behaviours by character, and which character each finder is.
struct Slots {
    by_id: HashMap<CharacterId, Slot>,
    finders: HashMap<FinderId, CharacterId>,
    next_finder: u64,
}

impl Slots {
    /// Character `id`'s slot, created (with a new [`FinderId`]) on first
    /// use.
    fn get(&mut self, id: CharacterId) -> &mut Slot {
        let Slots {
            by_id,
            finders,
            next_finder,
        } = self;
        by_id.entry(id).or_insert_with(|| {
            let finder = FinderId(*next_finder);
            *next_finder += 1;
            finders.insert(finder, id);
            Slot {
                behavior: PathFindBehavior2::new(finder),
                mover: None,
                pending: VecDeque::new(),
            }
        })
    }

    /// [`Self::get`] with `chr`'s queued results delivered first.
    fn flushed(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        seating: &dyn SeatingHooks,
    ) -> &mut Slot {
        let slot = self.get(id);
        slot.flush(chr, seating);
        slot
    }
}

/// `PathfindNative.instance` plus every character's `PathFindBehavior2`.
pub struct PathHost {
    service: PathfindService,
    slots: Slots,
}

impl PathHost {
    /// A host around a pathfinder (`PathfindNative.init`).
    pub fn new(service: PathfindService) -> Self {
        PathHost {
            service,
            slots: Slots {
                by_id: HashMap::new(),
                finders: HashMap::new(),
                next_finder: 1,
            },
        }
    }

    /// The pathfinder (chunk, square and vehicle updates go through it).
    pub fn service(&self) -> &PathfindService {
        &self.service
    }

    /// The pathfinder, mutably.
    pub fn service_mut(&mut self) -> &mut PathfindService {
        &mut self.service
    }

    /// `getPathFindBehavior2()` of character `id`, if it has one yet (a
    /// behaviour nobody has used reads like a new one: no goal, target
    /// 0, cancelled).
    pub fn behavior(&self, id: CharacterId) -> Option<&PathFindBehavior2> {
        self.slots.by_id.get(&id).map(|s| &s.behavior)
    }

    /// `getPathFindBehavior2()` of character `id`, created on first use.
    pub fn behavior_mut(&mut self, id: CharacterId) -> &mut PathFindBehavior2 {
        &mut self.slots.get(id).behavior
    }

    /// Character `id`'s behaviour and the pathfinder, for the goals without
    /// a method here (`pathToNearest`, vehicles, furniture, corpses).
    pub fn behavior_and_service(
        &mut self,
        id: CharacterId,
    ) -> (&mut PathFindBehavior2, &mut PathfindService) {
        (&mut self.slots.get(id).behavior, &mut self.service)
    }

    /// The character a finder belongs to.
    pub fn character_of(&self, finder: FinderId) -> Option<CharacterId> {
        self.slots.finders.get(&finder).copied()
    }

    /// Whether results for `id` are waiting for [`Self::deliver`].
    pub fn has_pending(&self, id: CharacterId) -> bool {
        self.slots
            .by_id
            .get(&id)
            .is_some_and(|s| !s.pending.is_empty())
    }

    /// Hands the queued results of `id` to it now.
    pub fn deliver(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        seating: &dyn SeatingHooks,
    ) {
        if let Some(slot) = self.slots.by_id.get_mut(&id) {
            slot.flush(chr, seating);
        }
    }

    /// The character left the world: its behaviour and queued results go,
    /// and its running request is cancelled.
    pub fn remove(&mut self, id: CharacterId) {
        if let Some(slot) = self.slots.by_id.remove(&id) {
            self.slots.finders.remove(&slot.behavior.finder);
            if let Some(mover) = slot.mover {
                self.service.cancel_request(mover);
            }
        }
    }

    // ----- the frame -----

    /// `PathfindNative.updateMain()`: vehicle changes go to the worker, and
    /// each finished request's result goes to its character's behaviour
    /// (`Succeeded` / `Failed`) through `movers`, in the order the worker
    /// finished them. See the module doc for characters `movers` cannot
    /// lend.
    pub fn update_main(
        &mut self,
        vehicles: &dyn VehicleSource,
        seating: &dyn SeatingHooks,
        movers: &mut dyn MoverAccess,
    ) {
        let mut delivered = Delivered::default();
        self.service.update_main(vehicles, &mut delivered);
        for (finder, mover, path) in delivered.results {
            let Some(&id) = self.slots.finders.get(&finder) else {
                continue;
            };
            let Some(slot) = self.slots.by_id.get_mut(&id) else {
                continue;
            };
            slot.mover = Some(mover);
            slot.pending.push_back(path);
            movers.with_mover(id, &mut |chr| slot.flush(chr, seating));
        }
    }

    /// One `PathfindNativeThread.updateThread()` pass.
    pub fn update_thread(&mut self) -> ThreadPass {
        self.service.update_thread()
    }

    // ----- PathFindBehavior2 for a character -----

    /// `getPathFindBehavior2().pathToLocation(x, y, z)`.
    pub fn path_to_location(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        x: i32,
        y: i32,
        z: i32,
    ) {
        let slot = self.slots.flushed(id, chr, &NoSeating);
        slot.behavior
            .path_to_location(chr, &mut self.service, x, y, z);
    }

    /// `getPathFindBehavior2().pathToLocationF(x, y, z)`.
    pub fn path_to_location_f(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        x: f32,
        y: f32,
        z: f32,
    ) {
        let slot = self.slots.flushed(id, chr, &NoSeating);
        slot.behavior
            .path_to_location_f(chr, &mut self.service, x, y, z);
    }

    /// `getPathFindBehavior2().pathToSound(x, y, z)`.
    pub fn path_to_sound(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        x: i32,
        y: i32,
        z: i32,
    ) {
        let slot = self.slots.flushed(id, chr, &NoSeating);
        slot.behavior.path_to_sound(chr, &mut self.service, x, y, z);
    }

    /// `getPathFindBehavior2().pathToCharacter(target)`. `target` is looked
    /// up in `env.characters`; false (and nothing done) when it is not
    /// there.
    pub fn path_to_character<G: GetSquare>(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        env: &PathEnv<'_, '_, '_, G>,
        target: MoverId,
    ) -> bool {
        let Some(target_view) = env.characters.character(target) else {
            return false;
        };
        let slot = self.slots.flushed(id, chr, env.seating);
        let mut ctx = context(&mut self.service, env);
        slot.behavior
            .path_to_character(chr, &mut ctx, target, target_view);
        true
    }

    /// `getPathFindBehavior2().update()`.
    pub fn path_update<G: GetSquare>(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        env: &PathEnv<'_, '_, '_, G>,
    ) -> BehaviorResult {
        self.path_update_speed(id, chr, env, 1.0)
    }

    /// `getPathFindBehavior2().update(speedMul)`.
    pub fn path_update_speed<G: GetSquare>(
        &mut self,
        id: CharacterId,
        chr: &mut dyn PathMover,
        env: &PathEnv<'_, '_, '_, G>,
        speed_mul: f32,
    ) -> BehaviorResult {
        let slot = self.slots.flushed(id, chr, env.seating);
        let mut ctx = context(&mut self.service, env);
        slot.behavior.update_speed(chr, &mut ctx, speed_mul)
    }

    /// `getPathFindBehavior2().cancel()`: only the flag, as in Java (the
    /// request runs on and its result is still delivered).
    pub fn path_cancel(&mut self, id: CharacterId, chr: &mut dyn PathMover) {
        self.slots.flushed(id, chr, &NoSeating).behavior.cancel();
    }

    /// `getPathFindBehavior2().walkingOnTheSpot.reset(x, y)`.
    pub fn walking_on_the_spot_reset(&mut self, id: CharacterId, x: f32, y: f32) {
        self.slots.get(id).behavior.walking_on_the_spot.reset(x, y);
    }

    /// `getPathFindBehavior2().walkingOnTheSpot.check(x, y)`.
    pub fn walking_on_the_spot_check(
        &mut self,
        id: CharacterId,
        x: f32,
        y: f32,
        world: &dyn PathWorldHooks,
    ) -> bool {
        self.slots
            .get(id)
            .behavior
            .walking_on_the_spot
            .check_xy(x, y, world)
    }

    // ----- collision queries (PolygonalMap2 / LosUtil on the main thread) -----

    /// The melee line test of `CombatManager` (`getValidMeleeTarget`):
    /// `LosUtil.lineClear(cell, (int)x0, (int)y0, z, (int)x1, (int)y1, z,
    /// false)` is neither `Blocked` nor `ClearThroughClosedDoor`. Both ends
    /// are on level `z`.
    pub fn line_clear<G: GetSquare>(
        map: &MainThreadMap<'_, '_, G>,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        z: i32,
    ) -> bool {
        let r = line_clear(
            &map.vision_env(),
            fastfloor(x0),
            fastfloor(y0),
            z,
            fastfloor(x1),
            fastfloor(y1),
            z,
            false,
        );
        !matches!(
            r,
            TestResults::Blocked | TestResults::ClearThroughClosedDoor
        )
    }

    /// `PolygonalMap2.lineClearCollide(x0, y0, x1, y1, z, ignore, flags)`:
    /// **true when blocked**. `ignore_vehicle` is the vehicle of the
    /// `ignore` object (`getVehicle()` of a character, or the vehicle
    /// itself). `flags`: 1 ignore doors, 2 close to walls, 4 check cost,
    /// 8 render (no effect).
    #[allow(clippy::too_many_arguments)]
    pub fn line_clear_collide<G: GetSquare>(
        map: &MainThreadMap<'_, '_, G>,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        z: i32,
        ignore_vehicle: Option<i16>,
        flags: i32,
    ) -> bool {
        LineClearCollideMain::new(map).is_not_clear(x0, y0, x1, y1, z, ignore_vehicle, flags)
    }

    /// `PolygonalMap2.lineClearCollide(x0, y0, x1, y1, z, ignore)` (the
    /// overload that ignores doors and keeps close to walls).
    #[allow(clippy::too_many_arguments)]
    pub fn line_clear_collide_default<G: GetSquare>(
        map: &MainThreadMap<'_, '_, G>,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        z: i32,
        ignore_vehicle: Option<i16>,
    ) -> bool {
        let flags = FLAG_IGNORE_DOORS | FLAG_CLOSE_TO_WALLS;
        Self::line_clear_collide(map, x0, y0, x1, y1, z, ignore_vehicle, flags)
    }

    /// `LosUtil.lineClear(cell, from, to, ignoreDoors)`.
    pub fn los_line_clear<G: GetSquare>(
        map: &MainThreadMap<'_, '_, G>,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        ignore_doors: bool,
    ) -> TestResults {
        line_clear(
            &map.vision_env(),
            from.0,
            from.1,
            from.2,
            to.0,
            to.1,
            to.2,
            ignore_doors,
        )
    }

    /// `LosUtil.lineClearCollide(from..., to..., ignoreDoors)`: **true when
    /// blocked**. As in Java, the walk starts at `to` (the second triple).
    pub fn los_line_clear_collide<G: GetSquare>(
        map: &MainThreadMap<'_, '_, G>,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
        ignore_doors: bool,
    ) -> bool {
        los_line_clear_collide(map, from.0, from.1, from.2, to.0, to.1, to.2, ignore_doors)
    }

    /// `PolygonalMap2.canStandAt(x, y, z, null, false, true)`.
    pub fn can_stand_at<G: GetSquare>(
        map: &MainThreadMap<'_, '_, G>,
        x: f32,
        y: f32,
        z: i32,
    ) -> bool {
        LineClearCollideMain::new(map).can_stand_at(x, y, z, None, false, true)
    }
}

/// A [`PathContext`] over the host's service.
fn context<'c, 'w, 'a, G: GetSquare>(
    service: &'c mut PathfindService,
    env: &PathEnv<'c, 'w, 'a, G>,
) -> PathContext<'c, 'w, 'a, G> {
    PathContext {
        service,
        map: env.map,
        world: env.world,
        vehicles: env.vehicles,
        seating: env.seating,
        characters: env.characters,
    }
}
