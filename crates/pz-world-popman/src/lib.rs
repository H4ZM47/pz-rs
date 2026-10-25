//! Glue between the zombie population manager (`pz-popman`) and the loaded
//! world (`pz-world`), so neither crate depends on the other's consumers.
//!
//! - [`WorldPopulationHost`] implements [`PopulationHost`]: grid squares and
//!   `getSquareForSittingZombie`'s square test come from an [`IsoCell`],
//!   random draws from the game's [`GameRand`], and everything done to a
//!   real `IsoZombie` goes to a [`ZombieOps`] the characters side supplies.
//! - [`SittingChecks`] is what that square test needs from outside the
//!   cell: the characters standing on a square (`movingObjects`) and
//!   `PolygonalMap2.lineClearCollide` from the pathfinder.
//! - [`player_chunk_maps`] reads the local players' chunk map origins from a
//!   [`ChunkStreamer`] for `ZombiePopulationManager::update_main`
//!   (`LoadedAreas.set`).

use pz_popman::manager::{PopulationHost, SquareInfo, SquarePlace};
use pz_world::sprite::{IsoFlagType, IsoSpriteManager};
use pz_world::streamer::ChunkStreamer;
use pz_world::{GameRand, IsoCell, IsoDirections, IsoGridSquare, IsoMetaGrid, WorldState};

/// What the population manager does to real zombies (`IsoZombie` and
/// `VirtualZombieManager`). The characters side implements it; each method
/// is the [`PopulationHost`] method of the same name.
pub trait ZombieOps {
    /// A handle to a real zombie.
    type Zombie: Copy + Eq;

    /// `VirtualZombieManager.createRealZombieAlways(outfit, dir, false,
    /// persistentId)` with `choices = [square]`.
    fn create_real_zombie(
        &mut self,
        outfit: i32,
        dir: IsoDirections,
        persistent_id: i32,
        square: (i32, i32, i32),
    ) -> Option<Self::Zombie>;
    /// `setX(x); setY(y)`.
    fn set_position(&mut self, z: Self::Zombie, x: f32, y: f32);
    /// `sitAgainstWall`'s effect: position, `setSitAgainstWall(true)`,
    /// `setDir`, `setForwardDirection` and the animation player's
    /// direction. `dir` is `None` when the square has no wall type.
    fn sit_against_wall(&mut self, z: Self::Zombie, x: f32, y: f32, dir: Option<IsoDirections>);
    /// `setHealth(health); sprite = legsSprite; setFakeDead(true)`.
    fn make_fake_dead(&mut self, z: Self::Zombie, health: f32);
    /// `setCrawler(true); setCanWalk(canWalk); setOnFloor(true);
    /// setFallOnFront(true); walkVariant = "ZombieWalk"; DoZombieStats()`.
    fn make_crawler(&mut self, z: Self::Zombie, can_walk: bool);
    /// `setCanCrawlUnderVehicle(v)`.
    fn set_can_crawl_under_vehicle(&mut self, z: Self::Zombie, v: bool);
    /// `setReanimatedForGrappleOnly(v)`.
    fn set_reanimated_for_grapple_only(&mut self, z: Self::Zombie, v: bool);
    /// `allowRepathDelay = -1; pathToLocation(x, y, 0)`.
    fn path_to(&mut self, z: Self::Zombie, x: i32, y: i32);
    /// `die()`.
    fn die(&mut self, z: Self::Zombie);
    /// `removeFromWorld(); removeFromSquare()`.
    fn remove_zombie(&mut self, z: Self::Zombie);
}

/// The parts of `getSquareForSittingZombie`'s square test that live outside
/// the cell.
pub trait SittingChecks {
    /// `!square.movingObjects.isEmpty()`: a character (player, zombie,
    /// animal) stands on the square.
    fn has_moving_objects(&self, x: i32, y: i32, z: i32) -> bool;
    /// `PolygonalMap2.instance.lineClearCollide(fromX, fromY, toX, toY, z,
    /// ignoreVehicle = null, bIgnoreDoors, bCloseToWalls)`: **true when
    /// blocked**. `pz-pathfind`'s `LineClearCollideMain::line_clear_collide`
    /// (over the same cell, with `pz_world::ObjectHooks` answering for
    /// thumpables, barricades and doors) is the implementation; pass
    /// `ignore_vehicle = None`.
    #[allow(clippy::too_many_arguments)]
    fn line_clear_collide(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        ignore_doors: bool,
        close_to_walls: bool,
    ) -> bool;
}

/// [`SittingChecks`] for a world with no characters on any square and no
/// pathfinding obstacles: every line is clear.
pub struct NoSittingObstacles;

impl SittingChecks for NoSittingObstacles {
    fn has_moving_objects(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
    fn line_clear_collide(
        &self,
        _from_x: f32,
        _from_y: f32,
        _to_x: f32,
        _to_y: f32,
        _z: i32,
        _ignore_doors: bool,
        _close_to_walls: bool,
    ) -> bool {
        false
    }
}

/// `IsoFlagType.WallN`, `WallW` and `WallNW`, looked up by name (they are
/// not among the flags `SpriteTables` resolves up front). A flag missing
/// from the table is never set on a square.
#[derive(Clone, Copy, Debug)]
pub struct WallFlags {
    pub wall_n: Option<IsoFlagType>,
    pub wall_w: Option<IsoFlagType>,
    pub wall_nw: Option<IsoFlagType>,
}

impl WallFlags {
    pub fn new(m: &IsoSpriteManager) -> Self {
        WallFlags {
            wall_n: m.tables.flag_from_string("WallN"),
            wall_w: m.tables.flag_from_string("WallW"),
            wall_nw: m.tables.flag_from_string("WallNW"),
        }
    }

    fn has(sq: &IsoGridSquare, f: Option<IsoFlagType>) -> bool {
        f.is_some_and(|f| sq.has(f))
    }
}

/// `IsoGridSquare.getWallType()`: bit 1 a north wall, 4 a west wall (both
/// for `WallNW`), 8 a west wall on the square to the east, 2 a north wall
/// on the square to the south. `nav[E]`/`nav[S]` are the cell's
/// neighbours.
pub fn wall_type(sq: &IsoGridSquare, cell: &IsoCell, f: &WallFlags) -> i32 {
    let mut ty = 0;
    if WallFlags::has(sq, f.wall_n) {
        ty |= 1;
    }
    if WallFlags::has(sq, f.wall_w) {
        ty |= 4;
    }
    if WallFlags::has(sq, f.wall_nw) {
        ty |= 5;
    }
    if let Some(e) = cell.get_grid_square(sq.x + 1, sq.y, sq.z) {
        if WallFlags::has(e, f.wall_w) || WallFlags::has(e, f.wall_nw) {
            ty |= 8;
        }
    }
    if let Some(s) = cell.get_grid_square(sq.x, sq.y + 1, sq.z) {
        if WallFlags::has(s, f.wall_n) || WallFlags::has(s, f.wall_nw) {
            ty |= 2;
        }
    }
    ty
}

/// The cached part of `IsoGridSquare.isFree(...)` (what it returns when
/// `movingObjects` is not counted or empty): stairs are free; otherwise a
/// solid floor with nothing `solid`, `solidtrans` or a tree on it.
///
/// The original caches this in `cachedIsFree` and clears the cache in
/// `RecalcProperties` and `AddStairs`; recomputing it is the same value.
pub fn is_free_ignoring_movers(sq: &IsoGridSquare, m: &IsoSpriteManager) -> bool {
    let f = &m.tables.flag;
    let ty = &m.tables.ty;
    let stairs = [
        ty.stairsBN,
        ty.stairsMN,
        ty.stairsTN,
        ty.stairsBW,
        ty.stairsMW,
        ty.stairsTW,
    ];
    if stairs.into_iter().any(|t| sq.has_type(t)) {
        return true;
    }
    !(sq.has(f.solid) || sq.has(f.solidtrans) || sq.has_type(ty.tree)) && sq.has(f.solidfloor)
}

/// `getBuilding().getDef()` and `getRoom().getRoomDef()` ids of `sq`
/// (`getRoom()` is null when `roomId == -1`; the building is the room's).
pub fn square_place(sq: &IsoGridSquare, meta: &IsoMetaGrid) -> SquarePlace {
    let room = sq.room_def(meta);
    SquarePlace {
        x: sq.x,
        y: sq.y,
        building: room
            .and_then(|r| r.building)
            .and_then(|b| meta.building_defs.get(b.0 as usize))
            .map(|b| b.id),
        room: room.map(|r| r.id),
    }
}

/// `sq` as the population manager reads it.
///
/// `solidFloorCached ? solidFloor : TreatAsSolidFloor()` is
/// `TreatAsSolidFloor()`: the cache holds that method's last result and is
/// cleared whenever the square is recalculated.
pub fn square_info(sq: &IsoGridSquare, meta: &IsoMetaGrid, m: &IsoSpriteManager) -> SquareInfo {
    SquareInfo {
        solid_floor: sq.treat_as_solid_floor(m),
        room_id: sq.room_id,
        place: square_place(sq, meta),
    }
}

/// The local players' chunk map origins and the grid width, as
/// `LoadedAreas.set` reads them for `ZombiePopulationManager::update_main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerChunkMaps {
    /// `(getWorldXMin(), getWorldYMin())` of each chunk map without
    /// `ignore`, in player order.
    pub origins: Vec<(i32, i32)>,
    /// `IsoChunkMap.chunkGridWidth`.
    pub width: i32,
}

/// [`PlayerChunkMaps`] of `streamer`: for each of `IsoPlayer.numPlayers`
/// chunk maps not marked `ignore`, `worldX - chunkGridWidth / 2` and
/// `worldY - chunkGridWidth / 2`.
pub fn player_chunk_maps(streamer: &ChunkStreamer) -> PlayerChunkMaps {
    let half = streamer.width / 2;
    let origins = (0..streamer.num_players())
        .map(|i| streamer.chunk_map(i))
        .filter(|cm| !cm.ignore)
        .map(|cm| (cm.world_x - half, cm.world_y - half))
        .collect();
    PlayerChunkMaps {
        origins,
        width: streamer.width,
    }
}

/// [`PopulationHost`] over the loaded cell.
pub struct WorldPopulationHost<'a, Z: ZombieOps> {
    pub cell: &'a IsoCell,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a IsoSpriteManager,
    /// Movers on squares and `lineClearCollide`.
    pub checks: &'a dyn SittingChecks,
    /// The game's `Rand`.
    pub rand: &'a mut dyn GameRand,
    /// The real zombies.
    pub zombies: &'a mut Z,
    pub wall_flags: WallFlags,
}

impl<'a, Z: ZombieOps> WorldPopulationHost<'a, Z> {
    pub fn new(
        cell: &'a IsoCell,
        meta: &'a IsoMetaGrid,
        sprites: &'a IsoSpriteManager,
        checks: &'a dyn SittingChecks,
        rand: &'a mut dyn GameRand,
        zombies: &'a mut Z,
    ) -> Self {
        WorldPopulationHost {
            cell,
            meta,
            sprites,
            checks,
            rand,
            zombies,
            wall_flags: WallFlags::new(sprites),
        }
    }

    /// [`new`](Self::new) over `state`'s cell, meta grid and sprites.
    pub fn from_state(
        state: &WorldState<'a>,
        checks: &'a dyn SittingChecks,
        rand: &'a mut dyn GameRand,
        zombies: &'a mut Z,
    ) -> Self {
        Self::new(state.cell, state.meta, state.sprites, checks, rand, zombies)
    }

    /// `square.isFree(true)`.
    pub fn is_free(&self, sq: &IsoGridSquare) -> bool {
        !self.checks.has_moving_objects(sq.x, sq.y, sq.z)
            && is_free_ignoring_movers(sq, self.sprites)
    }

    /// `square.getWallType()`.
    pub fn wall_type(&self, sq: &IsoGridSquare) -> i32 {
        wall_type(sq, self.cell, &self.wall_flags)
    }
}

impl<Z: ZombieOps> PopulationHost for WorldPopulationHost<'_, Z> {
    type Zombie = Z::Zombie;

    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        let sq = self.cell.get_grid_square(x, y, z)?;
        Some(square_info(sq, self.meta, self.sprites))
    }

    /// The test in `getSquareForSittingZombie`, in the original's order.
    fn sitting_wall_type(&self, x: i32, y: i32, z: i32, from_x: f32, from_y: f32) -> Option<i32> {
        let sq = self.cell.get_grid_square(x, y, z)?;
        if !self.is_free(sq) || square_place(sq, self.meta).building.is_some() {
            return None;
        }
        let wall = self.wall_type(sq);
        if wall == 0 {
            return None;
        }
        let (to_x, to_y) = (sq.x as f32 + 0.5, sq.y as f32 + 0.5);
        if self
            .checks
            .line_clear_collide(from_x, from_y, to_x, to_y, sq.z, false, true)
        {
            return None;
        }
        Some(wall)
    }

    fn rand(&mut self) -> &mut dyn GameRand {
        self.rand
    }

    fn create_real_zombie(
        &mut self,
        outfit: i32,
        dir: IsoDirections,
        persistent_id: i32,
        square: (i32, i32, i32),
    ) -> Option<Self::Zombie> {
        self.zombies
            .create_real_zombie(outfit, dir, persistent_id, square)
    }
    fn set_position(&mut self, z: Self::Zombie, x: f32, y: f32) {
        self.zombies.set_position(z, x, y);
    }
    fn sit_against_wall(&mut self, z: Self::Zombie, x: f32, y: f32, dir: Option<IsoDirections>) {
        self.zombies.sit_against_wall(z, x, y, dir);
    }
    fn make_fake_dead(&mut self, z: Self::Zombie, health: f32) {
        self.zombies.make_fake_dead(z, health);
    }
    fn make_crawler(&mut self, z: Self::Zombie, can_walk: bool) {
        self.zombies.make_crawler(z, can_walk);
    }
    fn set_can_crawl_under_vehicle(&mut self, z: Self::Zombie, v: bool) {
        self.zombies.set_can_crawl_under_vehicle(z, v);
    }
    fn set_reanimated_for_grapple_only(&mut self, z: Self::Zombie, v: bool) {
        self.zombies.set_reanimated_for_grapple_only(z, v);
    }
    fn path_to(&mut self, z: Self::Zombie, x: i32, y: i32) {
        self.zombies.path_to(z, x, y);
    }
    fn die(&mut self, z: Self::Zombie) {
        self.zombies.die(z);
    }
    fn remove_zombie(&mut self, z: Self::Zombie) {
        self.zombies.remove_zombie(z);
    }
}

#[cfg(test)]
mod tests;
