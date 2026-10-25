//! Read-only `IsoGridSquare` queries the states need: `isFree`,
//! `isSameStaircase`, `getGridSneakModifier` and the sheet-rope tests, and
//! `isBlockedTo`, `getSpecialWall`, `getSpecialSolid` and
//! `testCollideSpecialObjects` through pz-world's `CollideEnv`.

use pz_world::java::{fastfloor, parse_int};
use pz_world::objects::{IsoObject, IsoThumpable, ObjectId};
use pz_world::sprite::IsoFlagType;
use pz_world::square::CollideEnv;
use pz_world::square::GetSquare;
use pz_world::{IsoCell, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, ObjectHooks};

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
    pub fn sq(&self, x: i32, y: i32, z: i32) -> Option<&'a IsoGridSquare> {
        self.cell.get_grid_square(x, y, z)
    }

    /// The square at `p`.
    pub fn at(&self, p: (i32, i32, i32)) -> Option<&'a IsoGridSquare> {
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

    /// `obj.getProperties()` (the sprite's), `None` without a sprite.
    pub fn props(&self, obj: &IsoObject) -> Option<&'a pz_world::sprite::PropertyContainer> {
        obj.sprite.map(|s| &self.sprites.sprites[s].properties)
    }

    /// `obj.getProperties().has(IsoFlagType.<name>)`.
    pub fn obj_has_flag(&self, obj: &IsoObject, name: &str) -> bool {
        match (self.props(obj), self.flag(name)) {
            (Some(p), Some(f)) => p.has(f),
            _ => false,
        }
    }

    /// `obj.getProperties().has(<property name>)`.
    pub fn obj_has_prop(&self, obj: &IsoObject, name: &str) -> bool {
        self.props(obj)
            .is_some_and(|p| p.has_name(&self.sprites.alias, name))
    }

    /// `obj.getProperties().get(<property name>)`.
    pub fn obj_prop(&self, obj: &IsoObject, name: &str) -> Option<&'a str> {
        self.props(obj)
            .and_then(|p| p.get(&self.sprites.alias, name))
    }

    /// The sprite name of `obj` (`getSprite().getName()`).
    pub fn sprite_name(&self, obj: &IsoObject) -> Option<&'a str> {
        obj.sprite
            .and_then(|s| self.sprites.sprites[s].name.as_deref())
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
        this: &'a IsoGridSquare,
        next: Option<&'a IsoGridSquare>,
    ) -> Option<(&'a IsoGridSquare, ObjectId)> {
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
        self.sq(x, y, z).is_some_and(|s| s.has_stairs(self.sprites))
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
    pub fn can_climb_down_sheet_rope(&self, sq: Option<&IsoGridSquare>) -> bool {
        let Some(mut sq) = sq else { return false };
        let start_z = sq.z;
        loop {
            if !self.is_sheet_rope_here(sq) || !self.can_climb_here(sq) {
                return false;
            }
            if sq.treat_as_solid_floor(self.sprites) {
                return sq.z < start_z;
            }
            match self.sq(sq.x, sq.y, fastfloor(sq.z as f32 - 1.0)) {
                Some(s) => sq = s,
                None => return false,
            }
        }
    }

    /// `HasStairsBelow()`.
    pub fn has_stairs_below(&self, sq: &IsoGridSquare) -> bool {
        self.sq(sq.x, sq.y, sq.z - 1)
            .is_some_and(|s| s.has_stairs(self.sprites))
    }

    /// `getSheetRope()`: the first object with `sheetRope`.
    pub fn sheet_rope_index(&self, sq: &IsoGridSquare) -> Option<usize> {
        sq.objects.iter().position(|o| o.borrow().sheet_rope)
    }
}

impl GetSquare for Env<'_> {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare> {
        self.cell.get_grid_square(x, y, z)
    }
}
