//! `IsoGridSquare.testCollideAdjacent(collideObject, x, y, z)` with the
//! mover, the special-object path it takes first
//! (`testCollideSpecialObjects`, `getSpecialWall`, `getSpecialSolid`,
//! `isBlockedTo`), the hoppable edge queries (`getHoppableTo`,
//! `isHoppableTo`, `getHoppable`, `IsoObject.isHoppable`,
//! `isTallHoppable`, `haveDoor`, `getBendableTo`) and what
//! `IsoMovingObject` does with them after a square-edge collision
//! (`checkBreakHoppable`, `checkHitHoppable`, `checkBreakBendableFence`).
//!
//! The mover is a [`CollideMover`]: the few `IsoMovingObject` /
//! `IsoGameCharacter` / `IsoZombie` / `IsoPlayer` / `IsoAnimal` fields
//! these read. The object a mover hits is reported in
//! [`CollideAdjacent::special`] for the caller's `collideWith(obj)`,
//! `setCollidedWithDoor(true)` and `setCollidedObject(obj)` (the
//! character's code, `IsoZombie.collideWith` among it).
//!
//! Not here: the multiplayer safehouse trespass check (`GameServer` /
//! `GameClient` only) and `useSlowCollision` (a debug toggle, off).

use crate::chunk::{IsoCell, IsoGridSquare};
use crate::meta_grid::IsoMetaGrid;
use crate::objects::{IsoObject, IsoThumpable, ObjectId, ObjectKind};
use crate::sprite::IsoSpriteManager;
use crate::world_state::ObjectHooks;

use super::{get_matrix_bit, is, is_valid_chunk, EdgeFacing, GridSquareEdge, IsoDirections};

/// What a mover is (`instanceof` in the original).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoverKind {
    /// `IsoZombie`.
    Zombie,
    /// `IsoPlayer` that is not an `IsoAnimal`.
    Player,
    /// `IsoAnimal` (an `IsoPlayer` subclass).
    Animal,
    /// Any other `IsoMovingObject` (vehicles, physics objects).
    #[default]
    Other,
}

impl MoverKind {
    /// `instanceof IsoPlayer` (animals included).
    pub fn is_player_class(self) -> bool {
        matches!(self, MoverKind::Player | MoverKind::Animal)
    }
}

/// The `IsoMovingObject.collidedN/S/W/E` flags after a collision pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollidedFlags {
    pub n: bool,
    pub s: bool,
    pub w: bool,
    pub e: bool,
}

impl CollidedFlags {
    /// The single edge the hoppable checks act on (`dir` of
    /// `checkBreakHoppable` / `checkBreakBendableFence`: the last match of
    /// W, N, S, E wins).
    pub fn single_edge(self) -> Option<IsoDirections> {
        let mut dir = None;
        if self.w && !self.n && !self.s {
            dir = Some(IsoDirections::W);
        }
        if self.n && !self.e && !self.w {
            dir = Some(IsoDirections::N);
        }
        if self.s && !self.e && !self.w {
            dir = Some(IsoDirections::S);
        }
        if self.e && !self.n && !self.s {
            dir = Some(IsoDirections::E);
        }
        dir
    }
}

/// The states the hoppable checks test with `isCurrentState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoverStates {
    /// `AttackState`.
    pub attack: bool,
    /// `StaggerBackState`.
    pub stagger_back: bool,
    /// `ClimbOverFenceState`.
    pub climb_over_fence: bool,
    /// `ClimbThroughWindowState`.
    pub climb_through_window: bool,
    /// `CrawlingZombieTurnState`.
    pub crawling_zombie_turn: bool,
    /// `AnimalAttackState`.
    pub animal_attack: bool,
    /// `AnimalClimbOverFenceState`.
    pub animal_climb_over_fence: bool,
}

/// A mover as the collision code reads it.
pub trait CollideMover {
    /// What the mover is.
    fn mover_kind(&self) -> MoverKind;
    /// `IsoPlayer.isNoClip()` (players and animals).
    fn is_no_clip(&self) -> bool {
        false
    }
    /// `shouldIgnoreCollisionWithSquare(sq)`: for a character, the square
    /// of the furniture it sits on, or `getPathFindBehavior2()
    /// .shouldIgnoreCollisionWithSquare(sq)` while it has a path. `None` is
    /// a square that does not exist.
    fn should_ignore_collision_with_square(&self, _sq: Option<(i32, i32, i32)>) -> bool {
        false
    }
    /// `IsoZombie.crawling`.
    fn is_crawling(&self) -> bool {
        false
    }
    /// `collidedN/S/W/E`.
    fn collided(&self) -> CollidedFlags {
        CollidedFlags::default()
    }
    /// The states the hoppable checks look at.
    fn states(&self) -> MoverStates {
        MoverStates::default()
    }
    /// `IsoAnimal.adef.canClimbFences`.
    fn can_climb_fences(&self) -> bool {
        false
    }
}

/// An object on a square.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SquareObject {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: ObjectId,
}

impl SquareObject {
    fn on(sq: &IsoGridSquare, id: ObjectId) -> Self {
        SquareObject {
            x: sq.x,
            y: sq.y,
            z: sq.z,
            id,
        }
    }
}

/// The special object `testCollideAdjacent` hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialCollide {
    /// `collideWith(obj)` / `setCollidedObject(obj)`.
    pub object: SquareObject,
    /// `setCollidedWithDoor(true)`: an `IsoDoor` or a door `IsoThumpable`.
    pub door: bool,
}

/// What `testCollideAdjacent(mover, x, y, z)` answers, with its side
/// effects for the caller to apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollideAdjacent {
    /// The return value.
    pub blocked: bool,
    /// The special object hit (the return value is then true).
    pub special: Option<SpecialCollide>,
    /// A player (not an animal) found the collide bit set: the original
    /// runs `RecalcAllWithNeighbours(true)` on the square and returns the
    /// bit after that ([`IsoCell::test_collide_adjacent_mover_mut`] does
    /// both).
    pub recalc: bool,
}

/// What `checkBreakHoppable`, `checkHitHoppable` and
/// `checkBreakBendableFence` ask of the mover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoppableAction {
    /// `zombie.setThumpTarget(obj)`.
    ThumpTarget(SquareObject),
    /// `climbOverFence(dir)` (zombies and animals).
    ClimbOverFence(IsoDirections),
}

/// The world parts the hoppable checks read that this crate does not
/// model.
pub trait HoppableHooks {
    /// `obj.getThumpableFor(zombie) != null`.
    fn thumpable_for(&self, _sq: &IsoGridSquare, _obj: &IsoObject) -> bool {
        false
    }
    /// `BentFences.getInstance().isEnabled()`.
    fn bent_fences_enabled(&self) -> bool {
        false
    }
    /// `BentFences.getInstance().isUnbentObject(obj, facing)`.
    fn is_unbent_object(&self, _obj: &IsoObject, _facing: EdgeFacing) -> bool {
        false
    }
}

/// [`HoppableHooks`] with every default.
pub struct NoHoppableHooks;

impl HoppableHooks for NoHoppableHooks {}

impl IsoObject {
    /// `isHoppable()`: `IsoDoor` (closed, by its closed sprite),
    /// `IsoThumpable` (a closed door's closed sprite, else the sprite, else
    /// the `isHoppable` field) and `IsoObject` (`getHoppableDirection()`).
    pub fn is_hoppable(&self, m: &IsoSpriteManager) -> bool {
        let f = &m.tables.flag;
        let hop = |s: Option<crate::sprite::SpriteIdx>| {
            s.is_some_and(|s| {
                let p = &m.sprites[s].properties;
                p.has(f.HoppableN) || p.has(f.HoppableW)
            })
        };
        match &self.kind {
            ObjectKind::Door(d) => !d.open && hop(d.closed_sprite),
            ObjectKind::Thumpable(t) => {
                if t.is_door && !t.open && t.closed_sprite.is_some() {
                    return hop(t.closed_sprite);
                }
                hop(self.sprite) || t.is_hoppable
            }
            _ => self.get_hoppable_direction(m).is_some(),
        }
    }

    /// `isTallHoppable()`: the sprite has `TallHoppableN` or
    /// `TallHoppableW` (an `IsoThumpable` closed door: its closed sprite).
    pub fn is_tall_hoppable(&self, m: &IsoSpriteManager) -> bool {
        let flags = [
            m.tables.flag_from_string("TallHoppableN"),
            m.tables.flag_from_string("TallHoppableW"),
        ];
        let tall = |s: Option<crate::sprite::SpriteIdx>| {
            s.is_some_and(|s| {
                let p = &m.sprites[s].properties;
                flags.into_iter().flatten().any(|f| p.has(f))
            })
        };
        if let ObjectKind::Thumpable(t) = &self.kind {
            if t.is_door && !t.open && t.closed_sprite.is_some() {
                return tall(t.closed_sprite);
            }
        }
        tall(self.sprite)
    }

    /// `isNorthHoppable()`.
    pub fn is_north_hoppable(&self, m: &IsoSpriteManager) -> bool {
        self.get_hoppable_direction(m) == Some(EdgeFacing::NorthSouth)
    }
}

impl IsoGridSquare {
    /// `haveDoor()`: an `IsoDoor` among the objects.
    pub fn have_door(&self) -> bool {
        self.objects
            .iter()
            .any(|o| matches!(o.kind, ObjectKind::Door(_)))
    }

    /// `isDiagonalTo(other)`.
    pub fn is_diagonal_to(&self, other: &IsoGridSquare) -> bool {
        other.x != self.x && other.y != self.y
    }

    /// `HasStairsNorth()` from the tile types.
    fn stairs_north(&self, m: &IsoSpriteManager) -> bool {
        let t = &m.tables.ty;
        self.has_type(t.stairsTN) || self.has_type(t.stairsMN) || self.has_type(t.stairsBN)
    }

    /// `HasStairsWest()` from the tile types.
    fn stairs_west(&self, m: &IsoSpriteManager) -> bool {
        let t = &m.tables.ty;
        self.has_type(t.stairsTW) || self.has_type(t.stairsMW) || self.has_type(t.stairsBW)
    }
}

/// The loaded squares and the tables the mover-aware queries read.
#[derive(Clone, Copy)]
pub struct CollideEnv<'a> {
    pub cell: &'a IsoCell,
    pub sprites: &'a IsoSpriteManager,
    pub meta: &'a IsoMetaGrid,
}

impl<'a> CollideEnv<'a> {
    pub fn new(cell: &'a IsoCell, sprites: &'a IsoSpriteManager, meta: &'a IsoMetaGrid) -> Self {
        CollideEnv {
            cell,
            sprites,
            meta,
        }
    }

    /// The barricade/thumpable hooks over this cell.
    pub fn hooks(&self) -> ObjectHooks<'a> {
        ObjectHooks::new(self.cell, self.sprites)
    }

    /// `getCell().getGridSquare(x, y, z)`.
    pub fn sq(&self, x: i32, y: i32, z: i32) -> Option<&'a IsoGridSquare> {
        self.cell.get_grid_square(x, y, z)
    }

    /// `isStairBlockedTo(other)`.
    pub fn is_stair_blocked_to(&self, a: &IsoGridSquare, b: Option<&IsoGridSquare>) -> bool {
        let Some(b) = b else { return false };
        let ty = &self.sprites.tables.ty;
        a.x > b.x && a.has_type(ty.stairsTW) || a.y > b.y && a.has_type(ty.stairsTN)
    }

    /// `isBlockedTo(other)`.
    pub fn is_blocked_to(&self, a: &IsoGridSquare, b: Option<&IsoGridSquare>) -> bool {
        let h = self.hooks();
        a.is_wall_to(b, self.sprites, self.cell)
            || a.is_window_blocked_to(b, self.cell, &h)
            || a.is_door_blocked_to(b, self.cell, &h)
            || self.is_stair_blocked_to(a, b)
    }

    /// `IsoThumpable.isThumpable()`: barricaded, or the `isThumpable` field.
    pub fn thumpable_is_thumpable(
        &self,
        sq: &IsoGridSquare,
        obj: &IsoObject,
        t: &IsoThumpable,
    ) -> bool {
        self.hooks().object_barricaded(sq, obj) || t.is_thumpable
    }

    fn has_prop(&self, obj: &IsoObject, name: &str) -> bool {
        obj.sprite.is_some_and(|s| {
            self.sprites.sprites[s]
                .properties
                .has_name(&self.sprites.alias, name)
        })
    }

    /// `getSpecialWall(north)`: the closed door, window, thumpable or window
    /// frame on the square's north or west edge.
    pub fn get_special_wall(&self, sq: &IsoGridSquare, north: bool) -> Option<ObjectId> {
        let hooks = self.hooks();
        for &id in sq.special_objects.iter().rev() {
            let Some(i) = sq.object_index(id) else {
                continue;
            };
            let o = &sq.objects[i];
            match &o.kind {
                ObjectKind::Thumpable(t) => {
                    let skip = t.is_stairs
                        || !self.thumpable_is_thumpable(sq, o, t)
                            && !hooks.object_is_window(o)
                            && !t.is_door
                        || t.is_door && t.open
                        || t.block_all_the_square
                        || t.can_pass_through
                            && o.sprite.is_some()
                            && o.container.is_some()
                            && self.has_prop(o, "IsHigh");
                    if !skip && north == t.north && !t.is_corner {
                        return Some(o.id);
                    }
                }
                ObjectKind::Window(w) if north == w.north => return Some(o.id),
                ObjectKind::Door(d) if north == d.north && !d.open => return Some(o.id),
                _ => {}
            }
        }
        let f = &self.sprites.tables.flag;
        if north && !sq.has(f.WindowN) || !north && !sq.has(f.WindowW) {
            return None;
        }
        sq.get_window_frame_facing(EdgeFacing::of_north(north))
            .map(|o| o.id)
    }

    /// `isAdjacentToWindow() || isAdjacentToHoppable()`.
    fn adjacent_to_window_or_hoppable(&self, sq: &IsoGridSquare) -> bool {
        let h = self.hooks();
        sq.is_adjacent_to_window(self.sprites, self.cell, &h)
            || sq.is_adjacent_to_hoppable(self.sprites, self.cell, &h)
    }

    /// `getSpecialSolid()`: a thumpable that blocks the whole square, or a
    /// moved thumpable object.
    pub fn get_special_solid(&self, sq: &IsoGridSquare) -> Option<ObjectId> {
        let f = &self.sprites.tables.flag;
        for &id in &sq.special_objects {
            let Some(i) = sq.object_index(id) else {
                continue;
            };
            let o = &sq.objects[i];
            let ObjectKind::Thumpable(t) = &o.kind else {
                continue;
            };
            if t.is_stairs || !self.thumpable_is_thumpable(sq, o, t) || !t.block_all_the_square {
                continue;
            }
            if o.sprite
                .is_some_and(|s| self.sprites.sprites[s].properties.has(f.solidtrans))
                && self.adjacent_to_window_or_hoppable(sq)
            {
                return None;
            }
            return Some(o.id);
        }
        for o in &sq.objects {
            if !o.moved_thumpable {
                continue;
            }
            if self.adjacent_to_window_or_hoppable(sq) {
                return None;
            }
            return Some(o.id);
        }
        None
    }

    /// `this.testCollideSpecialObjects(next)`: the door, window, window
    /// frame or thumpable between `this` and a neighbouring square, with the
    /// square it is on.
    pub fn test_collide_special_objects(
        &self,
        this: &'a IsoGridSquare,
        next: Option<&'a IsoGridSquare>,
    ) -> Option<(&'a IsoGridSquare, ObjectId)> {
        let next = next?;
        if is(next, this) {
            return None;
        }
        let ty = &self.sprites.tables.ty;
        let wall =
            |s: &'a IsoGridSquare, north: bool| self.get_special_wall(s, north).map(|o| (s, o));
        let solid = |s: &'a IsoGridSquare| self.get_special_solid(s).map(|o| (s, o));
        let blocked = |a: &IsoGridSquare, b: &IsoGridSquare| self.is_blocked_to(a, Some(b));
        let (x, y, z) = (this.x, this.y, this.z);
        if next.x < x && next.y == y {
            if next.z == z && this.has_type(ty.stairsTW) {
                return None;
            }
            if next.z == z && this.has_sloped_surface_to_level_above(IsoDirections::W) {
                return None;
            }
            if let Some(o) = wall(this, false) {
                return Some(o);
            }
            if blocked(this, next) {
                return None;
            }
            return solid(next);
        }
        if next.x == x && next.y < y {
            if next.z == z && this.has_type(ty.stairsTN) {
                return None;
            }
            if next.z == z && this.has_sloped_surface_to_level_above(IsoDirections::N) {
                return None;
            }
            if let Some(o) = wall(this, true) {
                return Some(o);
            }
            if blocked(this, next) {
                return None;
            }
            return solid(next);
        }
        if next.x > x && next.y == y {
            if let Some(o) = wall(next, false) {
                return Some(o);
            }
            if blocked(this, next) {
                return None;
            }
            return solid(next);
        }
        if next.x == x && next.y > y {
            if let Some(o) = wall(next, true) {
                return Some(o);
            }
            if blocked(this, next) {
                return None;
            }
            return solid(next);
        }
        if next.x < x && next.y < y {
            if let Some(o) = wall(this, true).or_else(|| wall(this, false)) {
                return Some(o);
            }
            let a = self.sq(x, y - 1, z);
            if let Some(a) = a.filter(|a| !blocked(this, a)) {
                if let Some(o) = solid(a).or_else(|| wall(a, false)) {
                    return Some(o);
                }
            }
            let b = self.sq(x - 1, y, z);
            if let Some(b) = b.filter(|b| !blocked(this, b)) {
                if let Some(o) = solid(b).or_else(|| wall(b, true)) {
                    return Some(o);
                }
            }
            let (Some(a), Some(b)) = (a, b) else {
                return None;
            };
            if blocked(this, a) || blocked(this, b) || blocked(a, next) || blocked(b, next) {
                return None;
            }
            return solid(next);
        }
        if next.x > x && next.y < y {
            if let Some(o) = wall(this, true) {
                return Some(o);
            }
            let a = self.sq(x, y - 1, z);
            if let Some(a) = a.filter(|a| !blocked(this, a)) {
                if let Some(o) = solid(a) {
                    return Some(o);
                }
            }
            let b = self.sq(x + 1, y, z);
            if let Some(b) = b {
                if let Some(o) = wall(b, false) {
                    return Some(o);
                }
                if !blocked(this, b) {
                    if let Some(o) = solid(b).or_else(|| wall(b, true)) {
                        return Some(o);
                    }
                }
            }
            let (Some(a), Some(b)) = (a, b) else {
                return None;
            };
            if blocked(this, a) || blocked(this, b) {
                return None;
            }
            if let Some(o) = wall(next, false) {
                return Some(o);
            }
            if blocked(a, next) || blocked(b, next) {
                return None;
            }
            return solid(next);
        }
        if next.x > x && next.y > y {
            let a = self.sq(x, y + 1, z);
            if let Some(a) = a {
                if let Some(o) = wall(a, true) {
                    return Some(o);
                }
                if !blocked(this, a) {
                    if let Some(o) = solid(a) {
                        return Some(o);
                    }
                }
            }
            let b = self.sq(x + 1, y, z);
            if let Some(b) = b {
                if let Some(o) = wall(b, false) {
                    return Some(o);
                }
                if !blocked(this, b) {
                    if let Some(o) = solid(b) {
                        return Some(o);
                    }
                }
            }
            let (Some(a), Some(b)) = (a, b) else {
                return None;
            };
            if blocked(this, a) || blocked(this, b) {
                return None;
            }
            if let Some(o) = wall(next, false).or_else(|| wall(next, true)) {
                return Some(o);
            }
            if blocked(a, next) || blocked(b, next) {
                return None;
            }
            return solid(next);
        }
        if next.x < x && next.y > y {
            if let Some(o) = wall(this, false) {
                return Some(o);
            }
            let a = self.sq(x, y + 1, z);
            if let Some(a) = a {
                if let Some(o) = wall(a, true) {
                    return Some(o);
                }
                if !blocked(this, a) {
                    if let Some(o) = solid(a) {
                        return Some(o);
                    }
                }
            }
            let b = self.sq(x - 1, y, z);
            if let Some(b) = b.filter(|b| !blocked(this, b)) {
                if let Some(o) = solid(b) {
                    return Some(o);
                }
            }
            let (Some(a), Some(b)) = (a, b) else {
                return None;
            };
            if blocked(this, a) || blocked(this, b) {
                return None;
            }
            if let Some(o) = wall(next, true) {
                return Some(o);
            }
            if blocked(a, next) || blocked(b, next) {
                return None;
            }
            return solid(next);
        }
        None
    }

    /// `this.testCollideAdjacent(mover, x, y, z)`.
    pub fn test_collide_adjacent(
        &self,
        this: &'a IsoGridSquare,
        mover: Option<&dyn CollideMover>,
        x: i32,
        y: i32,
        z: i32,
    ) -> CollideAdjacent {
        let hit = |blocked| CollideAdjacent {
            blocked,
            ..Default::default()
        };
        if mover.is_some_and(|m| m.mover_kind().is_player_class() && m.is_no_clip()) {
            return hit(false);
        }
        if this.collide_matrix == -1 {
            return hit(true);
        }
        if !(-1..=1).contains(&x) || !(-1..=1).contains(&y) || !(-1..=1).contains(&z) {
            return hit(true);
        }
        if !is_valid_chunk(self.meta, (this.x + x) / 8, (this.y + y) / 8) {
            return hit(true);
        }
        let sq = self.sq(this.x + x, this.y + y, this.z + z);
        if let Some(m) = mover {
            if m.should_ignore_collision_with_square(sq.map(|s| (s.x, s.y, s.z))) {
                return hit(false);
            }
            if let Some(sq) = sq {
                if let Some((osq, id)) = self.test_collide_special_objects(this, Some(sq)) {
                    let door = osq
                        .object_index(id)
                        .is_some_and(|i| match &osq.objects[i].kind {
                            ObjectKind::Door(_) => true,
                            ObjectKind::Thumpable(t) => t.is_door,
                            _ => false,
                        });
                    return CollideAdjacent {
                        blocked: true,
                        special: Some(SpecialCollide {
                            object: SquareObject::on(osq, id),
                            door,
                        }),
                        recalc: false,
                    };
                }
            }
        }
        let bit = get_matrix_bit(this.collide_matrix, x + 1, y + 1, z + 1);
        CollideAdjacent {
            blocked: bit,
            special: None,
            recalc: bit && mover.is_some_and(|m| m.mover_kind() == MoverKind::Player),
        }
    }

    /// `getHoppableOrWindowFrame(alongEdge)`.
    pub fn get_hoppable_or_window_frame_edge(
        &self,
        sq: &'a IsoGridSquare,
        edge: GridSquareEdge,
    ) -> Option<(&'a IsoGridSquare, ObjectId)> {
        let m = self.sprites;
        sq.get_edge_element(edge, self.cell, &|s: &'a IsoGridSquare, f| {
            s.get_hoppable_or_window_frame_facing(f, m)
                .map(|o| (s, o.id))
        })
    }

    /// `getHoppable(north)` (`getHoppableOrWindowFrame(facing)` on this
    /// square).
    pub fn get_hoppable(&self, sq: &IsoGridSquare, north: bool) -> Option<ObjectId> {
        sq.get_hoppable_or_window_frame_facing(EdgeFacing::of_north(north), self.sprites)
            .map(|o| o.id)
    }

    /// `isHoppable(alongEdge)`.
    pub fn is_hoppable_edge(&self, sq: &'a IsoGridSquare, edge: GridSquareEdge) -> bool {
        self.get_hoppable_or_window_frame_edge(sq, edge).is_some()
    }

    /// `getHoppableTo(next)`: the hoppable object or window frame on the
    /// edge between the squares.
    pub fn get_hoppable_to(
        &self,
        sq: &'a IsoGridSquare,
        next: Option<&'a IsoGridSquare>,
    ) -> Option<(&'a IsoGridSquare, ObjectId)> {
        let m = self.sprites;
        sq.get_edge_element_to(next, self.cell, &|s: &'a IsoGridSquare, f| {
            s.get_hoppable_or_window_frame_facing(f, m)
                .map(|o| (s, o.id))
        })
    }

    /// `isHoppableTo(other)`: not diagonal, and a hoppable object or window
    /// frame on the edge between the squares.
    pub fn is_hoppable_to(&self, sq: &IsoGridSquare, other: Option<&IsoGridSquare>) -> bool {
        let Some(o) = other else { return false };
        let m = self.sprites;
        !sq.is_diagonal_to(o)
            && sq.is_edge_element_to(Some(o), self.cell, &|s, f| {
                s.get_hoppable_or_window_frame_facing(f, m).is_some()
            })
    }

    /// `getBendableTo(next)`: the unbent fence on the edge between the
    /// squares (`BentFences.isUnbentObject`).
    pub fn get_bendable_to(
        &self,
        sq: &'a IsoGridSquare,
        next: Option<&'a IsoGridSquare>,
        hooks: &dyn HoppableHooks,
    ) -> Option<(&'a IsoGridSquare, ObjectId)> {
        sq.get_edge_element_to(next, self.cell, &|s: &'a IsoGridSquare, f| {
            s.objects
                .iter()
                .find(|o| hooks.is_unbent_object(o, f))
                .map(|o| (s, o.id))
        })
    }

    /// `checkBreakHoppable()`, `checkHitHoppable()` and
    /// `checkBreakBendableFence(current)`, in that order, after a
    /// square-edge collision from `last` (`IsoMovingObject.last`, which the
    /// collision has made `current`).
    pub fn hoppable_actions(
        &self,
        mover: &dyn CollideMover,
        last: &'a IsoGridSquare,
        hooks: &dyn HoppableHooks,
    ) -> Vec<HoppableAction> {
        let mut out = Vec::new();
        let kind = mover.mover_kind();
        let st = mover.states();
        let c = mover.collided();
        let adj = |d: IsoDirections| last.get_adjacent_square(d, self.cell);
        let m = self.sprites;
        let f = &m.tables.flag;
        // checkBreakHoppable
        if kind == MoverKind::Zombie
            && mover.is_crawling()
            && !(st.attack || st.stagger_back || st.crawling_zombie_turn)
        {
            if let Some(dir) = c.single_edge() {
                if let Some((hsq, id)) = self.get_hoppable_to(last, adj(dir)) {
                    let i = hsq.object_index(id).unwrap();
                    let o = &hsq.objects[i];
                    let target = match &o.kind {
                        ObjectKind::Thumpable(t) if !self.thumpable_is_thumpable(hsq, o, t) => true,
                        _ => hooks.thumpable_for(hsq, o),
                    };
                    if target {
                        out.push(HoppableAction::ThumpTarget(SquareObject::on(hsq, id)));
                    }
                }
            }
        }
        // checkHitHoppable
        if kind == MoverKind::Animal {
            if mover.can_climb_fences() && !(st.animal_attack || st.animal_climb_over_fence) {
                if c.w && !c.n && !c.s && last.has(f.HoppableW) {
                    out.push(HoppableAction::ClimbOverFence(IsoDirections::W));
                }
                if c.n && !c.e && !c.w && last.has(f.HoppableN) {
                    out.push(HoppableAction::ClimbOverFence(IsoDirections::N));
                }
                if c.s && !c.e && !c.w && adj(IsoDirections::S).is_some_and(|s| s.has(f.HoppableN))
                {
                    out.push(HoppableAction::ClimbOverFence(IsoDirections::S));
                }
                if c.e && !c.n && !c.s && adj(IsoDirections::E).is_some_and(|e| e.has(f.HoppableW))
                {
                    out.push(HoppableAction::ClimbOverFence(IsoDirections::E));
                }
            }
        } else if kind == MoverKind::Zombie
            && !mover.is_crawling()
            && !(st.attack || st.stagger_back || st.climb_over_fence || st.climb_through_window)
        {
            if c.w && !c.n && !c.s {
                let w = adj(IsoDirections::W);
                if last.has(f.HoppableW)
                    && !last.stairs_north(m)
                    && !w.is_some_and(|w| w.stairs_north(m))
                {
                    out.push(HoppableAction::ClimbOverFence(IsoDirections::W));
                }
            }
            if c.n && !c.e && !c.w {
                let n = adj(IsoDirections::N);
                if last.has(f.HoppableN)
                    && !last.stairs_west(m)
                    && !n.is_some_and(|n| n.stairs_west(m))
                {
                    out.push(HoppableAction::ClimbOverFence(IsoDirections::N));
                }
            }
            if c.s && !c.e && !c.w {
                if let Some(s) = adj(IsoDirections::S) {
                    if s.has(f.HoppableN) && !last.stairs_west(m) && !s.stairs_west(m) {
                        out.push(HoppableAction::ClimbOverFence(IsoDirections::S));
                    }
                }
            }
            if c.e && !c.n && !c.s {
                if let Some(e) = adj(IsoDirections::E) {
                    if e.has(f.HoppableW) && !last.stairs_north(m) && !e.stairs_north(m) {
                        out.push(HoppableAction::ClimbOverFence(IsoDirections::E));
                    }
                }
            }
        }
        // checkBreakBendableFence
        if kind == MoverKind::Zombie && !(st.attack || st.stagger_back || st.crawling_zombie_turn) {
            if let Some(dir) = c.single_edge() {
                let bend = self.get_bendable_to(last, adj(dir), hooks);
                if hooks.bent_fences_enabled() {
                    if let Some((bsq, id)) = bend {
                        let o = &bsq.objects[bsq.object_index(id).unwrap()];
                        let target = match &o.kind {
                            ObjectKind::Thumpable(t) if !self.thumpable_is_thumpable(bsq, o, t) => {
                                true
                            }
                            _ => hooks.thumpable_for(bsq, o),
                        };
                        if target {
                            out.push(HoppableAction::ThumpTarget(SquareObject::on(bsq, id)));
                        }
                    }
                }
            }
        }
        out
    }
}

impl IsoCell {
    /// `testCollideAdjacent(mover, x, y, z)` on the square at `from`,
    /// running the `RecalcAllWithNeighbours(true)` a player's set collide
    /// bit asks for and reading the bit again (`None` when there is no such
    /// square; the original never asks for one).
    pub fn test_collide_adjacent_mover_mut(
        &mut self,
        from: (i32, i32, i32),
        mover: Option<&dyn CollideMover>,
        d: (i32, i32, i32),
        meta: &IsoMetaGrid,
        m: &IsoSpriteManager,
    ) -> Option<CollideAdjacent> {
        let r = {
            let env = CollideEnv::new(self, m, meta);
            let sq = self.get_grid_square(from.0, from.1, from.2)?;
            env.test_collide_adjacent(sq, mover, d.0, d.1, d.2)
        };
        if !r.recalc {
            return Some(r);
        }
        self.recalc_all_with_neighbours(from.0, from.1, from.2, meta, m);
        let sq = self.get_grid_square(from.0, from.1, from.2)?;
        Some(CollideAdjacent {
            blocked: get_matrix_bit(sq.collide_matrix, d.0 + 1, d.1 + 1, d.2 + 1),
            ..r
        })
    }
}

/// `IsoWindow.load`'s side effects on the shared sprites, for a window
/// read from a save (the chunk loader holds the sprites read only): the
/// closed sprite gets `cutN`/`windowN` (or `cutW`/`windowW`), and the
/// smashed, open and glass-removed sprites take the closed sprite's
/// properties (`AddProperties`) and tile type. Run it for every loaded
/// window in load order ([`IsoChunk::apply_window_load_sprites`]) before
/// the squares' properties are recalculated; until then smashed and open
/// windows lack the `WindowN`/`WindowW` the hoppable and collision code
/// reads.
pub fn window_load_sprites(m: &mut IsoSpriteManager, w: &crate::objects::IsoWindow) {
    let Some(closed) = w.closed_sprite else {
        return;
    };
    let f = m.tables.flag;
    let p = &mut m.sprites[closed].properties;
    if w.north {
        p.set_flag(f.cutN);
        p.set_flag(f.windowN);
    } else {
        p.set_flag(f.cutW);
        p.set_flag(f.windowW);
    }
    for other in [w.smashed_sprite, w.open_sprite, w.glass_removed_sprite] {
        if let Some(o) = other.filter(|&o| o != closed) {
            let props = m.sprites[closed].properties.clone();
            let tile_type = m.sprites[closed].tile_type;
            m.sprites[o].properties.add_properties(&props);
            m.sprites[o].tile_type = tile_type;
        }
    }
}

impl crate::chunk::IsoChunk {
    /// [`window_load_sprites`] for the chunk's windows, in the order
    /// `IsoChunk.load` reads them (level, then square, then object).
    pub fn apply_window_load_sprites(&self, m: &mut IsoSpriteManager) {
        for z in self.min_level..=self.max_level {
            for x in 0..8 {
                for y in 0..8 {
                    let Some(sq) = self.get_grid_square(x, y, z) else {
                        continue;
                    };
                    for o in &sq.objects {
                        if let ObjectKind::Window(w) = &o.kind {
                            window_load_sprites(m, w);
                        }
                    }
                }
            }
        }
    }
}
