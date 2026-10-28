//! Read-only `IsoGridSquare` queries the states need: `isFree`,
//! `isSameStaircase`, `getGridSneakModifier` and the sheet-rope tests, and
//! `isBlockedTo`, `getSpecialWall`, `getSpecialSolid` and
//! `testCollideSpecialObjects` through pz-world's `CollideEnv`.
//!
//! Existing synchronous state interfaces require unborrowed object/sprite
//! bodies and resolved metadata. The `try_` sprite reads preserve typed host
//! errors for fallible callers; legacy wrappers never convert those errors
//! into an absent sprite or a tile-arena projection.

use pz_entity::runtime_load::HostResult;
use pz_world::java::{fastfloor, parse_int};
use pz_world::objects::{IsoObject, IsoThumpable, ObjectId};
use pz_world::sprite::IsoFlagType;
use pz_world::square::CollideEnv;
use pz_world::square::GetSquare;
use pz_world::{IsoCell, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, ObjectHooks, SquareRef};

/// The loaded cell and the tables its queries read.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    pub cell: &'a IsoCell,
    pub sprites: &'a IsoSpriteManager,
    pub meta: &'a IsoMetaGrid,
}

impl<'a> Env<'a> {
    pub fn new(cell: &'a IsoCell, sprites: &'a IsoSpriteManager, meta: &'a IsoMetaGrid) -> Self {
        Env {
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
    pub fn sq(&self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        self.cell.get_grid_square(x, y, z)
    }

    /// The square at `p`.
    pub fn at(&self, p: (i32, i32, i32)) -> Option<SquareRef> {
        self.sq(p.0, p.1, p.2)
    }

    /// An `IsoFlagType` by name (`None` when the game's table has no such
    /// constant, which no property can then carry).
    pub fn flag(&self, name: &str) -> Option<IsoFlagType> {
        self.sprites.tables.flag_from_string(name)
    }

    /// `square.has(IsoFlagType.<name>)`.
    pub fn sq_has(&self, sq: &IsoGridSquare, name: &str) -> bool {
        self.flag(name).is_some_and(|f| sq.has(f))
    }

    /// Current sprite metadata, captured only for this query phase. A live
    /// sprite is read from its actual body, including when absent from the
    /// tile arena; an unresolved archive ID is a host capability error.
    pub fn try_metadata(&self, obj: &IsoObject) -> HostResult<Option<pz_world::sprite::IsoSprite>> {
        Ok(obj
            .sprite
            .try_read(self.sprites)?
            .map(|sprite| (*sprite).clone()))
    }

    /// Legacy synchronous state queries require unborrowed sprite bodies and
    /// resolved metadata. They have no host-error return carrier.
    pub fn metadata(&self, obj: &IsoObject) -> Option<pz_world::sprite::IsoSprite> {
        self.try_metadata(obj)
            .expect("state query requires unborrowed, resolved sprite metadata")
    }

    /// `obj.getProperties()` as owned phase data; no borrow survives a caller
    /// callback or a later mutation of the same actual sprite.
    pub fn try_props(
        &self,
        obj: &IsoObject,
    ) -> HostResult<Option<pz_world::sprite::PropertyContainer>> {
        Ok(obj
            .sprite
            .try_read(self.sprites)?
            .map(|sprite| sprite.properties.clone()))
    }

    /// Infallible legacy wrapper with the same precondition as `metadata`.
    pub fn props(&self, obj: &IsoObject) -> Option<pz_world::sprite::PropertyContainer> {
        self.try_props(obj)
            .expect("state query requires unborrowed, resolved sprite properties")
    }

    pub fn try_obj_has_flag(&self, obj: &IsoObject, name: &str) -> HostResult<bool> {
        Ok(match (self.try_props(obj)?, self.flag(name)) {
            (Some(p), Some(f)) => p.has(f),
            _ => false,
        })
    }

    /// `obj.getProperties().has(IsoFlagType.<name>)`.
    pub fn obj_has_flag(&self, obj: &IsoObject, name: &str) -> bool {
        self.try_obj_has_flag(obj, name)
            .expect("state query requires unborrowed, resolved sprite properties")
    }

    pub fn try_obj_has_prop(&self, obj: &IsoObject, name: &str) -> HostResult<bool> {
        Ok(self
            .try_props(obj)?
            .is_some_and(|p| p.has_name(&self.sprites.alias, name)))
    }

    /// `obj.getProperties().has(<property name>)`.
    pub fn obj_has_prop(&self, obj: &IsoObject, name: &str) -> bool {
        self.try_obj_has_prop(obj, name)
            .expect("state query requires unborrowed, resolved sprite properties")
    }

    pub fn try_obj_prop(&self, obj: &IsoObject, name: &str) -> HostResult<Option<String>> {
        Ok(self
            .try_props(obj)?
            .and_then(|p| p.get(&self.sprites.alias, name).map(str::to_owned)))
    }

    /// `obj.getProperties().get(<property name>)` as an owned phase value.
    pub fn obj_prop(&self, obj: &IsoObject, name: &str) -> Option<String> {
        self.try_obj_prop(obj, name)
            .expect("state query requires unborrowed, resolved sprite properties")
    }

    pub fn try_sprite_name(&self, obj: &IsoObject) -> HostResult<Option<String>> {
        Ok(obj
            .sprite
            .try_read(self.sprites)?
            .and_then(|sprite| sprite.name.clone()))
    }

    /// The current sprite name of `obj` (`getSprite().getName()`). The legacy
    /// wrapper requires unborrowed, resolved sprite metadata.
    pub fn sprite_name(&self, obj: &IsoObject) -> Option<String> {
        self.try_sprite_name(obj)
            .expect("state query requires unborrowed, resolved sprite name")
    }

    /// The mover-aware collision queries over the same cell
    /// (`pz_world::square::CollideEnv`).
    pub fn collide(&self) -> CollideEnv<'a> {
        CollideEnv::new(self.cell, self.sprites, self.meta)
    }

    /// `isStairBlockedTo(other)`.
    pub fn is_stair_blocked_to(&self, a: &IsoGridSquare, b: Option<&IsoGridSquare>) -> bool {
        self.collide().is_stair_blocked_to(a, b)
    }

    /// `isBlockedTo(other)`.
    pub fn is_blocked_to(&self, a: &IsoGridSquare, b: Option<&IsoGridSquare>) -> bool {
        self.collide().is_blocked_to(a, b)
    }

    /// `isSomethingTo(other)`.
    pub fn is_something_to(&self, a: &IsoGridSquare, b: &IsoGridSquare) -> bool {
        a.is_wall_to(Some(b), self.sprites, self.cell)
            || a.is_window_to(Some(b), self.sprites, self.cell)
            || a.is_door_to(Some(b), self.sprites, self.cell)
    }

    /// `IsoThumpable.isThumpable()`: barricaded, or the `isThumpable` field.
    pub fn thumpable_is_thumpable(
        &self,
        sq: &IsoGridSquare,
        obj: &IsoObject,
        t: &IsoThumpable,
    ) -> bool {
        self.collide().thumpable_is_thumpable(sq, obj, t)
    }

    /// `IsoObject.isWindow()` (the sprite's `WindowN`/`WindowW`).
    pub fn is_window(&self, obj: &IsoObject) -> bool {
        self.hooks().object_is_window(obj)
    }

    /// `getSpecialWall(north)`: the closed door, window, thumpable or window
    /// frame on the square's north or west edge.
    pub fn get_special_wall(&self, sq: &IsoGridSquare, north: bool) -> Option<ObjectId> {
        self.collide().get_special_wall(sq, north)
    }

    /// `getSpecialSolid()`: a thumpable that blocks the whole square, or a
    /// moved thumpable object.
    pub fn get_special_solid(&self, sq: &IsoGridSquare) -> Option<ObjectId> {
        self.collide().get_special_solid(sq)
    }

    /// `this.testCollideSpecialObjects(next)`: the door, window, window
    /// frame or thumpable between `this` and a neighbouring square, with the
    /// square it is on.
    pub fn test_collide_special_objects(
        &self,
        this: &SquareRef,
        next: Option<&SquareRef>,
    ) -> Option<(SquareRef, ObjectId)> {
        self.collide().test_collide_special_objects(this, next)
    }

    /// `isFree(false)` (without the cache): no solid object or tree, a
    /// solid floor, or stairs.
    pub fn is_free(&self, sq: &IsoGridSquare) -> bool {
        let f = &self.sprites.tables.flag;
        let ty = &self.sprites.tables.ty;
        let mut free = true;
        if sq.has(f.solid) || sq.has(f.solidtrans) || sq.has_type(ty.tree) {
            free = false;
        }
        if !sq.has(f.solidfloor) {
            free = false;
        }
        if [
            ty.stairsBN,
            ty.stairsMN,
            ty.stairsTN,
            ty.stairsBW,
            ty.stairsMW,
            ty.stairsTW,
        ]
        .into_iter()
        .any(|t| sq.has_type(t))
        {
            free = true;
        }
        free
    }

    /// `isSameStaircase(x, y, z)`.
    pub fn is_same_staircase(&self, sq: &IsoGridSquare, x: i32, y: i32, z: i32) -> bool {
        if z != sq.z {
            return false;
        }
        let ty = &self.sprites.tables.ty;
        let (mut min_x, mut min_y) = (sq.x, sq.y);
        let (mut max_x, mut max_y) = (min_x, min_y);
        min_x -= 1;
        min_y -= 1;
        if sq.has_type(ty.stairsTN) {
            max_y += 2;
        } else if sq.has_type(ty.stairsMN) {
            max_y += 1;
        } else if sq.has_type(ty.stairsBN) {
            min_y -= 2;
        } else if sq.has_type(ty.stairsTW) {
            max_x += 2;
        } else if sq.has_type(ty.stairsMW) {
            max_x += 1;
        } else if sq.has_type(ty.stairsBW) {
            min_x -= 2;
        } else {
            return false;
        }
        if x < min_x || y < min_y || x > max_x || y > max_y {
            return false;
        }
        self.sq(x, y, z)
            .is_some_and(|s| s.borrow().has_stairs(self.sprites))
    }

    /// `getGridSneakModifier(onlySolidTrans)`.
    pub fn grid_sneak_modifier(&self, sq: &IsoGridSquare, only_solid_trans: bool) -> f32 {
        let f = &self.sprites.tables.flag;
        if !only_solid_trans {
            if let Some(v) = sq.properties.get(&self.sprites.alias, "CloseSneakBonus") {
                // `Integer.parseInt` throws on a malformed value; such a
                // tile would crash the original.
                return parse_int(v).unwrap_or(0) as f32 / 100.0;
            }
            if [
                f.collideN, f.collideW, f.WindowN, f.WindowW, f.doorN, f.doorW,
            ]
            .into_iter()
            .any(|fl| sq.has(fl))
            {
                return 8.0;
            }
        } else if sq.has(f.solidtrans) {
            return 4.0;
        }
        1.0
    }

    /// `IsoWindow.isSheetRopeHere(sq)`.
    pub fn is_sheet_rope_here(&self, sq: &IsoGridSquare) -> bool {
        [
            "climbSheetTopW",
            "climbSheetTopN",
            "climbSheetTopE",
            "climbSheetTopS",
            "climbSheetW",
            "climbSheetN",
            "climbSheetE",
            "climbSheetS",
        ]
        .into_iter()
        .any(|n| self.sq_has(sq, n))
    }

    /// `IsoWindow.canClimbHere(sq)`.
    pub fn can_climb_here(&self, sq: &IsoGridSquare) -> bool {
        let ty = &self.sprites.tables.ty;
        if sq.has(self.sprites.tables.flag.solid) {
            return false;
        }
        ![
            ty.stairsBN,
            ty.stairsMN,
            ty.stairsTN,
            ty.stairsBW,
            ty.stairsMW,
            ty.stairsTW,
        ]
        .into_iter()
        .any(|t| sq.has_type(t))
    }

    /// `IsoGameCharacter.canClimbDownSheetRope(sq)`.
    pub fn can_climb_down_sheet_rope(&self, square: Option<&IsoGridSquare>) -> bool {
        let Some(square) = square else { return false };
        let start_z = square.z;
        let check = |sq: &IsoGridSquare| {
            if !self.is_sheet_rope_here(sq) || !self.can_climb_here(sq) {
                Some(false)
            } else if sq.treat_as_solid_floor(self.sprites) {
                Some(sq.z < start_z)
            } else {
                None
            }
        };
        if let Some(result) = check(square) {
            return result;
        }
        let mut next = (square.x, square.y, fastfloor(square.z as f32 - 1.0));
        loop {
            let Some(owner) = self.sq(next.0, next.1, next.2) else {
                return false;
            };
            let sq = owner.borrow();
            if let Some(result) = check(&sq) {
                return result;
            }
            next = (sq.x, sq.y, fastfloor(sq.z as f32 - 1.0));
        }
    }

    /// `HasStairsBelow()`.
    pub fn has_stairs_below(&self, sq: &IsoGridSquare) -> bool {
        self.sq(sq.x, sq.y, sq.z - 1)
            .is_some_and(|s| s.borrow().has_stairs(self.sprites))
    }

    /// `getSheetRope()`: the first object with `sheetRope`.
    pub fn sheet_rope_index(&self, sq: &IsoGridSquare) -> Option<usize> {
        sq.objects.iter().position(|o| o.borrow().sheet_rope)
    }
}

impl GetSquare for Env<'_> {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<SquareRef> {
        self.cell.get_grid_square(x, y, z)
    }
}
