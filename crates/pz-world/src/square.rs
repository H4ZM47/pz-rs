//! `IsoGridSquare`'s derived state and the queries built on it:
//! `RecalcProperties` (the merged property container, the exterior rule),
//! `haveRoof`, the collide/path/vision matrices (`CalculateCollide`,
//! `CalculateVisionBlocked`, `RecalcAllWithNeighbours`) and the edge
//! queries (`isWallTo`, `isWindowTo`, `isDoorTo`, `isWindowBlockedTo`,
//! `isDoorBlockedTo`, `getWindowTo`, `testCollideAdjacent`).
//!
//! Neighbours are looked up through a [`GetSquare`] (`IsoGridSquare.GetSquare`):
//! [`IsoCell`] looks across chunks, [`ChunkGetter`] only inside one chunk
//! (what chunk loading uses before the chunk is in the world).
//!
//! Barricades and `IsoThumpable` are not in this crate yet; the queries that
//! read them go through [`SquareHooks`], whose defaults report "no
//! barricade" and "no thumpable".

use crate::chunk::{IsoCell, IsoChunk, IsoGridSquare, MAX_LEVEL, MIN_LEVEL};
use crate::meta_grid::IsoMetaGrid;
use crate::objects::{IsoObject, ObjectKind};
use crate::sprite::{IsoFlagType, IsoObjectType, IsoSpriteManager, PropertyContainer};

/// `IsoDirections`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IsoDirections {
    N,
    NW,
    W,
    SW,
    S,
    SE,
    E,
    NE,
}

impl IsoDirections {
    /// `IsoDirections.values()`.
    pub const ALL: [IsoDirections; 8] = [
        IsoDirections::N,
        IsoDirections::NW,
        IsoDirections::W,
        IsoDirections::SW,
        IsoDirections::S,
        IsoDirections::SE,
        IsoDirections::E,
        IsoDirections::NE,
    ];

    /// `dx()`, `dy()`.
    pub fn delta(self) -> (i32, i32) {
        match self {
            IsoDirections::N => (0, -1),
            IsoDirections::NW => (-1, -1),
            IsoDirections::W => (-1, 0),
            IsoDirections::SW => (-1, 1),
            IsoDirections::S => (0, 1),
            IsoDirections::SE => (1, 1),
            IsoDirections::E => (1, 0),
            IsoDirections::NE => (1, -1),
        }
    }

    /// `Rot180()`.
    pub fn rot180(self) -> IsoDirections {
        IsoDirections::ALL[(self as usize + 4) & 7]
    }

    /// `IsoDirections.fromString`: `valueOf(trim().toUpperCase())`. Java
    /// throws for other names; this returns `None`.
    pub fn from_name(s: &str) -> Option<IsoDirections> {
        let u = s.trim().to_uppercase();
        IsoDirections::ALL
            .into_iter()
            .find(|d| format!("{d:?}") == u)
    }
}

/// `GridSquareEdge`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridSquareEdge {
    North,
    West,
    South,
    East,
}

/// `GridSquareEdgeFacingDirection`: `NORTH_SOUTH` is a north edge object
/// (`getNorth()`), `EAST_WEST` a west one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeFacing {
    NorthSouth,
    EastWest,
}

impl EdgeFacing {
    /// `getGridSquareEdgeFacingDirection()` of an edge element.
    pub fn of_north(north: bool) -> EdgeFacing {
        if north {
            EdgeFacing::NorthSouth
        } else {
            EdgeFacing::EastWest
        }
    }
}

/// `IsoGridSquare.GetSquare`.
pub trait GetSquare {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare>;
}

impl GetSquare for IsoCell {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare> {
        IsoCell::get_grid_square(self, x, y, z)
    }
}

/// `IsoChunk.ChunkGetter`: squares of one chunk only.
pub struct ChunkGetter<'a>(pub &'a IsoChunk);

impl GetSquare for ChunkGetter<'_> {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare> {
        let (x, y) = (x - self.0.wx * 8, y - self.0.wy * 8);
        if (0..8).contains(&x) && (0..8).contains(&y) && (-32..=31).contains(&z) {
            self.0.get_grid_square(x, y, z)
        } else {
            None
        }
    }
}

/// What the square queries read from objects this crate does not model
/// yet. Waiting on the world thread's barricade (`IsoBarricade`) and
/// `IsoThumpable` work; the defaults behave as if neither exists.
pub trait SquareHooks {
    /// `isBarricaded()` of an `IsoDoor` or `IsoWindow` on `sq`.
    fn is_barricaded(&self, _sq: &IsoGridSquare, _obj: &IsoObject) -> bool {
        false
    }

    /// Whether `sq` has an `IsoThumpable` with `isBlockedDoor(facing)`
    /// (a closed or barricaded thumpable door on that edge).
    fn has_blocked_thumpable_door(&self, _sq: &IsoGridSquare, _facing: EdgeFacing) -> bool {
        false
    }
}

/// [`SquareHooks`] with every default: no barricades, no thumpables.
pub struct NoSquareHooks;

impl SquareHooks for NoSquareHooks {}

/// `getMatrixBit(matrix, x, y, z)`.
pub fn get_matrix_bit(matrix: i32, x: i32, y: i32, z: i32) -> bool {
    (matrix >> (x + y * 3 + z * 9)) & 1 != 0
}

/// `setMatrixBit(matrix, x, y, z, val)`.
pub fn set_matrix_bit(matrix: i32, x: i32, y: i32, z: i32, val: bool) -> i32 {
    let bit = 1 << (x + y * 3 + z * 9);
    if val {
        matrix | bit
    } else {
        matrix & !bit
    }
}

fn same(a: Option<&IsoGridSquare>, b: Option<&IsoGridSquare>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => (a.x, a.y, a.z) == (b.x, b.y, b.z),
        _ => false,
    }
}

fn is(a: &IsoGridSquare, b: &IsoGridSquare) -> bool {
    (a.x, a.y, a.z) == (b.x, b.y, b.z)
}

impl IsoGridSquare {
    /// `RecalcProperties()`: rebuilds [`IsoGridSquare::properties`] from the
    /// objects' sprite properties and applies the derived-flag rules
    /// (exterior, water/`solidtrans`, `transparentFloor`, hoppable walls,
    /// cut-wall transparency). Rain drops/splashes are render-side and not
    /// tracked here.
    pub fn recalc_properties(&mut self, m: &IsoSpriteManager) {
        let t = &m.tables;
        let f = &t.flag;
        let fuel_amount = if self.properties.has_name(&m.alias, "fuelAmount") {
            self.properties
                .get(&m.alias, "fuelAmount")
                .map(str::to_string)
        } else {
            None
        };
        let mut props = PropertyContainer::default();
        self.has_types = 0;
        self.has_tree = false;
        let mut non_water_solid_trans = false;
        let mut non_water_solid_floor = false;
        let mut non_transparent_solid_floor = false;
        let mut non_hoppable_collide_n = false;
        let mut non_hoppable_collide_w = false;
        let mut non_transparent_cut_n = false;
        let mut non_transparent_cut_w = false;
        let mut force_render = false;
        for obj in &self.objects {
            let Some(s) = obj.sprite else { continue };
            let sp = &m.sprites[s];
            let p = &sp.properties;
            if p.has(f.blueprint) {
                continue;
            }
            if sp.force_render {
                force_render = true;
            }
            if sp.tile_type == t.ty.tree {
                self.has_tree = true;
            }
            self.has_types |= 1u64 << (sp.tile_type.index() & 63);
            props.add_properties(p);
            if p.has(f.water) {
                non_water_solid_floor = false;
            } else {
                if !non_water_solid_floor && p.has(f.solidfloor) {
                    non_water_solid_floor = true;
                }
                if !non_water_solid_trans && p.has(f.solidtrans) {
                    non_water_solid_trans = true;
                }
                if !non_transparent_solid_floor && p.has(f.solidfloor) && !p.has(f.transparentFloor)
                {
                    non_transparent_solid_floor = true;
                }
            }
            if !non_hoppable_collide_n && p.has(f.collideN) && !p.has(f.HoppableN) {
                non_hoppable_collide_n = true;
            }
            if !non_hoppable_collide_w && p.has(f.collideW) && !p.has(f.HoppableW) {
                non_hoppable_collide_w = true;
            }
            if !non_transparent_cut_n && p.has(f.cutN) && !p.has(f.transparentN) && !p.has(f.WallSE)
            {
                non_transparent_cut_n = true;
            }
            if !non_transparent_cut_w && p.has(f.cutW) && !p.has(f.transparentW) && !p.has(f.WallSE)
            {
                non_transparent_cut_w = true;
            }
        }
        if self.room_id != -1 || self.have_roof {
            props.unset_flag(f.exterior);
        } else {
            props.set_flag(f.exterior);
        }
        if let Some(v) = fuel_amount {
            props.set_raw(&m.alias, "fuelAmount", &v);
        }
        if force_render {
            props.set_flag(f.forceRender);
        }
        if self.burnt_out {
            props.set_flag(f.burntOut);
        }
        if !non_water_solid_trans && non_water_solid_floor && props.has(f.water) {
            props.unset_flag(f.solidtrans);
        }
        if non_transparent_solid_floor && props.has(f.transparentFloor) {
            props.unset_flag(f.transparentFloor);
        }
        if non_hoppable_collide_n && props.has(f.HoppableN) {
            props.unset_flag(f.canPathN);
            props.unset_flag(f.HoppableN);
        }
        if non_hoppable_collide_w && props.has(f.HoppableW) {
            props.unset_flag(f.canPathW);
            props.unset_flag(f.HoppableW);
        }
        if non_transparent_cut_n && props.has(f.transparentN) {
            props.unset_flag(f.transparentN);
        }
        if non_transparent_cut_w && props.has(f.transparentW) {
            props.unset_flag(f.transparentW);
        }
        self.surface = props.surface_info(&m.alias);
        self.sloped_surface_direction = props
            .get(&m.alias, "SlopedSurfaceDirection")
            .and_then(IsoDirections::from_name);
        self.properties = props;
    }

    /// `has(IsoFlagType)`.
    pub fn has(&self, flag: IsoFlagType) -> bool {
        self.properties.has(flag)
    }

    /// `has(IsoObjectType)` (`hasTypes`).
    pub fn has_type(&self, ty: IsoObjectType) -> bool {
        self.has_types & (1u64 << (ty.index() & 63)) != 0
    }

    /// `hasRainBlockingTile()`.
    pub fn has_rain_blocking_tile(&self, m: &IsoSpriteManager) -> bool {
        self.has(m.tables.flag.solidfloor) || self.has(m.tables.flag.BlockRain)
    }

    /// `isExterior()` / `has(IsoFlagType.exterior)`.
    pub fn is_exterior(&self, m: &IsoSpriteManager) -> bool {
        self.has(m.tables.flag.exterior)
    }

    /// `getVisionMatrix(dx, dy, dz)`: vision to that neighbour is blocked.
    pub fn get_vision_matrix(&self, dx: i32, dy: i32, dz: i32) -> bool {
        get_matrix_bit(self.vision_matrix, dx + 1, dy + 1, dz + 1)
    }

    /// `getStairs()`: `None` is `IsoObjectType.MAX`.
    pub fn get_stairs(&self, m: &IsoSpriteManager) -> Option<IsoObjectType> {
        let ty = &m.tables.ty;
        [
            ty.stairsTW,
            ty.stairsTN,
            ty.stairsMW,
            ty.stairsMN,
            ty.stairsBW,
            ty.stairsBN,
        ]
        .into_iter()
        .find(|&s| self.has_type(s))
    }

    /// `HasStairsNorth()`.
    pub fn has_stairs_north(&self, m: &IsoSpriteManager) -> bool {
        let ty = &m.tables.ty;
        self.has_type(ty.stairsTN) || self.has_type(ty.stairsMN) || self.has_type(ty.stairsBN)
    }

    /// `HasStairsWest()`.
    pub fn has_stairs_west(&self, m: &IsoSpriteManager) -> bool {
        let ty = &m.tables.ty;
        self.has_type(ty.stairsTW) || self.has_type(ty.stairsMW) || self.has_type(ty.stairsBW)
    }

    /// `HasStairs()`.
    pub fn has_stairs(&self, m: &IsoSpriteManager) -> bool {
        self.has_stairs_north(m) || self.has_stairs_west(m)
    }

    /// `HasElevatedFloor()`.
    pub fn has_elevated_floor(&self, m: &IsoSpriteManager) -> bool {
        let ty = &m.tables.ty;
        self.has_type(ty.stairsTN)
            || self.has_type(ty.stairsMN)
            || self.has_type(ty.stairsTW)
            || self.has_type(ty.stairsMW)
    }

    /// `hasSlopedSurface()`.
    pub fn has_sloped_surface(&self) -> bool {
        self.sloped_surface_direction.is_some()
    }

    /// `TreatAsSolidFloor()` (uncached).
    pub fn treat_as_solid_floor(&self, m: &IsoSpriteManager) -> bool {
        self.has(m.tables.flag.solidfloor) || self.has_stairs(m) || self.has_sloped_surface()
    }

    /// `hasFloorOverWater()`.
    pub fn has_floor_over_water(&self, m: &IsoSpriteManager) -> bool {
        self.has(m.tables.flag.water) && !self.has(m.tables.flag.solidtrans)
    }

    /// `getSlopedSurfaceHeight(IsoDirections edge)`.
    pub fn get_sloped_surface_height(&self, edge: IsoDirections) -> f32 {
        let Some(dir) = self.sloped_surface_direction else {
            return 0.0;
        };
        if dir == edge {
            self.surface.sloped_surface_height_max as f32 / 100.0
        } else if dir.rot180() == edge {
            self.surface.sloped_surface_height_min as f32 / 100.0
        } else {
            -1.0
        }
    }

    /// `hasSlopedSurfaceToLevelAbove(dir)`.
    pub fn has_sloped_surface_to_level_above(&self, dir: IsoDirections) -> bool {
        self.sloped_surface_direction.is_some() && self.get_sloped_surface_height(dir) == 1.0
    }

    /// `getAdjacentSquare(dir)`: `nav[dir]`, the same-level neighbour.
    pub fn get_adjacent_square<'a>(
        &self,
        dir: IsoDirections,
        g: &'a impl GetSquare,
    ) -> Option<&'a IsoGridSquare> {
        let (dx, dy) = dir.delta();
        g.get_grid_square(self.x + dx, self.y + dy, self.z)
    }

    /// `isSlopedSurfaceEdgeBlocked(edge)`.
    pub fn is_sloped_surface_edge_blocked(&self, edge: IsoDirections, g: &impl GetSquare) -> bool {
        if self.sloped_surface_direction.is_none() {
            return false;
        }
        let Some(sq2) = self.get_adjacent_square(edge, g) else {
            return true;
        };
        self.get_sloped_surface_height(edge) != sq2.get_sloped_surface_height(edge.rot180())
    }

    /// `CalculateVisionBlocked(gridSquare, getter)`.
    pub fn calculate_vision_blocked(
        &self,
        other: Option<&IsoGridSquare>,
        m: &IsoSpriteManager,
        g: &impl GetSquare,
    ) -> bool {
        let Some(o) = other else { return false };
        let f = &m.tables.flag;
        if (o.x - self.x).abs() > 1 || (o.y - self.y).abs() > 1 {
            return true;
        }
        let test_w = o.x < self.x;
        let test_n = o.y < self.y;
        let test_e = o.x > self.x;
        let test_s = o.y > self.y;
        if o.has(f.trans) || self.has(f.trans) {
            return false;
        }
        let opaque_floor = |s: &IsoGridSquare| s.has(f.solidfloor) && !s.has(f.transparentFloor);
        if self.z != o.z {
            if o.z > self.z {
                if opaque_floor(o) || self.has(f.noStart) {
                    return true;
                }
                match g.get_grid_square(self.x, self.y, o.z) {
                    None => return false,
                    Some(sq) if opaque_floor(sq) => return true,
                    _ => {}
                }
            } else {
                if opaque_floor(self) || self.has(f.noStart) {
                    return true;
                }
                match g.get_grid_square(o.x, o.y, self.z) {
                    None => return false,
                    Some(sq) if opaque_floor(sq) => return true,
                    _ => {}
                }
            }
        }
        let col_n =
            test_n && self.has(f.collideN) && !self.has(f.transparentN) && !self.has(f.doorN);
        let col_w =
            test_w && self.has(f.collideW) && !self.has(f.transparentW) && !self.has(f.doorW);
        let col_s = test_s && o.has(f.collideN) && !o.has(f.transparentN) && !o.has(f.doorN);
        let col_e = test_e && o.has(f.collideW) && !o.has(f.transparentW) && !o.has(f.doorW);
        if col_n || col_w || col_s || col_e {
            return true;
        }
        let diag = o.x != self.x && o.y != self.y;
        if o.has(f.solid) || o.has(f.blocksight) {
            return true;
        }
        if diag {
            let a = g.get_grid_square(self.x, o.y, self.z);
            let b = g.get_grid_square(o.x, self.y, self.z);
            if self.calculate_vision_blocked(a, m, g)
                || self.calculate_vision_blocked(b, m, g)
                || o.calculate_vision_blocked(a, m, g)
                || o.calculate_vision_blocked(b, m, g)
            {
                return true;
            }
        }
        false
    }

    /// `CalculateCollide(gridSquare, bVision, bPathfind, bIgnoreSolidTrans,
    /// bIgnoreSolid, getter)`.
    #[allow(clippy::too_many_arguments)]
    pub fn calculate_collide(
        &self,
        other: Option<&IsoGridSquare>,
        vision: bool,
        pathfind: bool,
        ignore_solid_trans: bool,
        ignore_solid: bool,
        m: &IsoSpriteManager,
        g: &impl GetSquare,
    ) -> bool {
        let _ = vision;
        let Some(o) = other else { return pathfind };
        let f = &m.tables.flag;
        let ty = &m.tables.ty;
        if self.has(f.water) && !self.has_floor_over_water(m)
            || o.has(f.water) && !o.has_floor_over_water(m)
        {
            return true;
        }
        let test_w = o.x < self.x;
        let test_n = o.y < self.y;
        let test_e = o.x > self.x;
        let test_s = o.y > self.y;
        if !ignore_solid && o.has(f.solid) {
            if self.has_type(ty.stairsTW)
                && !pathfind
                && o.x < self.x
                && o.y == self.y
                && o.z == self.z
            {
                return false;
            }
            return !self.has_type(ty.stairsTN)
                || pathfind
                || o.x != self.x
                || o.y >= self.y
                || o.z != self.z;
        }
        if !ignore_solid_trans && o.has(f.solidtrans) {
            if self.has_type(ty.stairsTW)
                && !pathfind
                && o.x < self.x
                && o.y == self.y
                && o.z == self.z
            {
                return false;
            }
            if self.has_type(ty.stairsTN)
                && !pathfind
                && o.x == self.x
                && o.y < self.y
                && o.z == self.z
            {
                return false;
            }
            let hop = |s: &IsoGridSquare| s.has(f.HoppableN) || s.has(f.HoppableW);
            let mut has_hoppable = hop(o);
            if !has_hoppable && g.get_grid_square(o.x, o.y + 1, self.z).is_some_and(hop) {
                has_hoppable = true;
            }
            if !has_hoppable && g.get_grid_square(o.x + 1, o.y, self.z).is_some_and(hop) {
                has_hoppable = true;
            }
            let mut has_window = o.has(f.windowW) || o.has(f.windowN);
            if !has_window && (o.has(f.WindowW) || o.has(f.WindowN)) {
                has_window = true;
            }
            if !has_window
                && g.get_grid_square(o.x, o.y + 1, self.z)
                    .is_some_and(|s| s.has(f.windowN) || s.has(f.WindowN))
            {
                has_window = true;
            }
            if !has_window
                && g.get_grid_square(o.x + 1, o.y, self.z)
                    .is_some_and(|s| s.has(f.windowW) || s.has(f.WindowW))
            {
                has_window = true;
            }
            if !has_window && !has_hoppable {
                return true;
            }
        }
        if o.x != self.x && o.y != self.y && self.z != o.z && pathfind {
            return true;
        }
        if pathfind && o.z < self.z && !self.treat_as_solid_floor(m) {
            return o.has_type(ty.stairsTN) || o.has_type(ty.stairsTW);
        }
        if pathfind && o.z == self.z {
            if o.x > self.x && o.y == self.y && o.has(f.windowW) {
                return false;
            }
            if o.y > self.y && o.x == self.x && o.has(f.windowN) {
                return false;
            }
            if o.x < self.x && o.y == self.y && self.has(f.windowW) {
                return false;
            }
            if o.y < self.y && o.x == self.x && self.has(f.windowN) {
                return false;
            }
        }
        if o.x > self.x && o.z < self.z && o.has_type(ty.stairsTW) {
            return false;
        }
        if o.y > self.y && o.z < self.z && o.has_type(ty.stairsTN) {
            return false;
        }
        let below_targ = g.get_grid_square(o.x, o.y, o.z - 1);
        let current = self.get_stairs(m);
        let target = o.get_stairs(m);
        let left_stairs = if self.x < o.x { current } else { target };
        let right_stairs = if self.x > o.x { current } else { target };
        let up_stairs = if self.y < o.y { current } else { target };
        let down_stairs = if self.y > o.y { current } else { target };
        let left_square = if self.x < o.x { self } else { o };
        let right_square = if self.x > o.x { self } else { o };
        let up_square = if self.y < o.y { self } else { o };
        let down_square = if self.y > o.y { self } else { o };
        let has_current = current.is_some();
        let has_target = target.is_some();
        let same_level = o.z == self.z;
        let diagonal = o.x != self.x && o.y != self.y;
        let at_top = current == Some(ty.stairsTN) || current == Some(ty.stairsTW);
        let targeting_top = target == Some(ty.stairsTN) || target == Some(ty.stairsTW);
        let top_stair_floor_transition = at_top && !has_target || !has_current && targeting_top;
        if same_level && self.is_wall_to(Some(o), m, g) && !top_stair_floor_transition {
            return true;
        }
        if has_current && has_target && same_level && target == current {
            let c = current.unwrap();
            let adjacent = if c == ty.stairsTN || c == ty.stairsMN || c == ty.stairsBN {
                self.x != o.x && self.y == o.y
            } else if c == ty.stairsTW || c == ty.stairsMW || c == ty.stairsBW {
                self.x == o.x && self.y != o.y
            } else {
                false
            };
            if adjacent {
                return false;
            }
        }
        if same_level && diagonal && has_current && has_target {
            // isNorthFacingSequence(up, down), isWestFacingSequence(left, right).
            let s = |t| Some(t);
            if up_stairs == s(ty.stairsTN) && down_stairs == s(ty.stairsMN)
                || up_stairs == s(ty.stairsMN) && down_stairs == s(ty.stairsBN)
            {
                return false;
            }
            if left_stairs == s(ty.stairsMW) && right_stairs == s(ty.stairsTW)
                || left_stairs == s(ty.stairsBW) && right_stairs == s(ty.stairsMW)
            {
                return false;
            }
        }
        if diagonal && (has_current || has_target) {
            let adjacent_has = |sq: &IsoGridSquare, dir: IsoDirections, t: IsoObjectType| {
                sq.get_adjacent_square(dir, g)
                    .is_some_and(|a| a.has_type(t))
            };
            // hasConnectingBNStair(up, left, upSquare)
            if same_level && up_stairs == Some(ty.stairsBN) {
                let dir = if left_stairs == up_stairs {
                    IsoDirections::E
                } else {
                    IsoDirections::W
                };
                if adjacent_has(up_square, dir, ty.stairsBN) {
                    return false;
                }
            }
            // hasConnectingBWStair(right, left, rightSquare)
            if same_level && right_stairs == Some(ty.stairsBW) {
                let dir = if left_stairs == right_stairs {
                    IsoDirections::S
                } else {
                    IsoDirections::N
                };
                if adjacent_has(right_square, dir, ty.stairsBW) {
                    return false;
                }
            }
            // hasConnectingTNStair(down, left, downSquare, upSquare)
            if down_stairs == Some(ty.stairsTN) && down_square.z == up_square.z - 1 {
                let dir = if left_stairs == down_stairs {
                    IsoDirections::E
                } else {
                    IsoDirections::W
                };
                if adjacent_has(down_square, dir, ty.stairsTN) {
                    return false;
                }
            }
            // hasConnectingTWStair(left, up, leftSquare, upSquare)
            if left_stairs == Some(ty.stairsTW) && left_square.z == up_square.z - 1 {
                let dir = if left_stairs == up_stairs {
                    IsoDirections::S
                } else {
                    IsoDirections::N
                };
                if adjacent_has(left_square, dir, ty.stairsTW) {
                    return false;
                }
            }
        }
        let below_lacks = |t: IsoObjectType| below_targ.is_none_or(|b| !b.has_type(t)) || pathfind;
        if o.x != self.x && o.z == self.z && o.has_type(ty.stairsTN) && below_lacks(ty.stairsTN) {
            return true;
        }
        if o.y > self.y
            && o.x == self.x
            && o.z == self.z
            && o.has_type(ty.stairsTN)
            && below_lacks(ty.stairsTN)
        {
            return true;
        }
        if o.x > self.x
            && o.y == self.y
            && o.z == self.z
            && o.has_type(ty.stairsTW)
            && below_lacks(ty.stairsTW)
        {
            return true;
        }
        if o.y != self.y && o.z == self.z && o.has_type(ty.stairsTW) && below_lacks(ty.stairsTW) {
            return true;
        }
        let same_z = o.z == self.z;
        if o.x != self.x && same_z && o.has_type(ty.stairsMN) {
            return true;
        }
        if o.y != self.y && same_z && o.has_type(ty.stairsMW) {
            return true;
        }
        if o.x != self.x && same_z && o.has_type(ty.stairsBN) {
            return true;
        }
        if o.y != self.y && same_z && o.has_type(ty.stairsBW) {
            return true;
        }
        if o.x != self.x && same_z && self.has_type(ty.stairsTN) {
            return true;
        }
        if o.y != self.y && same_z && self.has_type(ty.stairsTW) {
            return true;
        }
        if o.x != self.x && same_z && self.has_type(ty.stairsMN) {
            return true;
        }
        if o.y != self.y && same_z && self.has_type(ty.stairsMW) {
            return true;
        }
        if o.x != self.x && same_z && self.has_type(ty.stairsBN) {
            return true;
        }
        if o.y != self.y && same_z && self.has_type(ty.stairsBW) {
            return true;
        }
        if o.y < self.y && o.x == self.x && o.z > self.z && self.has_type(ty.stairsTN) {
            return false;
        }
        if o.x < self.x && o.y == self.y && o.z > self.z && self.has_type(ty.stairsTW) {
            return false;
        }
        if o.y > self.y && o.x == self.x && o.z < self.z && o.has_type(ty.stairsTN) {
            return false;
        }
        if o.x > self.x && o.y == self.y && o.z < self.z && o.has_type(ty.stairsTW) {
            return false;
        }
        if same_z {
            use IsoDirections::*;
            let (s1, s2) = (self, o);
            let edge = |d: IsoDirections, back: IsoDirections| {
                !s1.has_sloped_surface_to_level_above(d)
                    && (s1.is_sloped_surface_edge_blocked(d, g)
                        || s2.is_sloped_surface_edge_blocked(back, g))
            };
            if s2.x == s1.x && s2.y == s1.y - 1 && edge(N, S) {
                return true;
            }
            if s2.x == s1.x && s2.y == s1.y + 1 && edge(S, N) {
                return true;
            }
            if s2.x == s1.x - 1 && s2.y == s1.y && edge(W, E) {
                return true;
            }
            if s2.x == s1.x + 1 && s2.y == s1.y && edge(E, W) {
                return true;
            }
        }
        if o.z > self.z {
            use IsoDirections::*;
            if o.y < self.y && o.x == self.x && self.has_sloped_surface_to_level_above(N) {
                return false;
            }
            if o.y > self.y && o.x == self.x && self.has_sloped_surface_to_level_above(S) {
                return false;
            }
            if o.x < self.x && o.y == self.y && self.has_sloped_surface_to_level_above(W) {
                return false;
            }
            if o.x > self.x && o.y == self.y && self.has_sloped_surface_to_level_above(E) {
                return false;
            }
        }
        if o.z < self.z {
            use IsoDirections::*;
            if o.y > self.y && o.x == self.x && o.has_sloped_surface_to_level_above(N) {
                return false;
            }
            if o.y < self.y && o.x == self.x && o.has_sloped_surface_to_level_above(S) {
                return false;
            }
            if o.x > self.x && o.y == self.y && o.has_sloped_surface_to_level_above(W) {
                return false;
            }
            if o.x < self.x && o.y == self.y && o.has_sloped_surface_to_level_above(E) {
                return false;
            }
        }
        if same_z && !o.treat_as_solid_floor(m) && pathfind {
            return true;
        }
        if same_z
            && !o.treat_as_solid_floor(m)
            && o.z > 0
            && g.get_grid_square(o.x, o.y, o.z - 1).is_none()
        {
            return true;
        }
        if self.z != o.z {
            return o.z >= self.z || o.x != self.x || o.y != self.y || self.treat_as_solid_floor(m);
        }
        let mut col_n = test_n && self.has(f.collideN);
        let mut col_w = test_w && self.has(f.collideW);
        let mut col_s = test_s && o.has(f.collideN);
        let mut col_e = test_e && o.has(f.collideW);
        if col_n && pathfind && self.has(f.canPathN) {
            col_n = false;
        }
        if col_w && pathfind && self.has(f.canPathW) {
            col_w = false;
        }
        if col_s && pathfind && o.has(f.canPathN) {
            col_s = false;
        }
        if col_e && pathfind && o.has(f.canPathW) {
            col_e = false;
        }
        if col_w && self.has_type(ty.stairsTW) && !pathfind {
            col_w = false;
        }
        if col_n && self.has_type(ty.stairsTN) && !pathfind {
            col_n = false;
        }
        if col_n || col_w || col_s || col_e {
            return true;
        }
        if o.x != self.x && o.y != self.y {
            let a = g.get_grid_square(self.x, o.y, self.z);
            let b = g.get_grid_square(o.x, self.y, self.z);
            let this = Some(self);
            if same(a, b) || same(a, this) || same(b, this) || same(a, Some(o)) || same(b, Some(o))
            {
                return true;
            }
            if let (Some(a), Some(b)) = (a, b) {
                if o.x == self.x + 1 && o.y == self.y + 1 && a.has(f.windowN) && b.has(f.windowW) {
                    return true;
                }
                if o.x == self.x - 1 && o.y == self.y - 1 && a.has(f.windowW) && b.has(f.windowN) {
                    return true;
                }
            }
            if self.calculate_collide(a, vision, pathfind, ignore_solid_trans, false, m, g)
                || self.calculate_collide(b, vision, pathfind, ignore_solid_trans, false, m, g)
                || o.calculate_collide(a, vision, pathfind, ignore_solid_trans, false, m, g)
                || o.calculate_collide(b, vision, pathfind, ignore_solid_trans, false, m, g)
            {
                return true;
            }
        }
        false
    }

    /// `testCollideAdjacent(null, x, y, z)`: the collide matrix bit, with
    /// the range and `isValidChunk` checks. The mover-specific parts
    /// (no-clip, `shouldIgnoreCollisionWithSquare`, special objects'
    /// `TestCollide`, safehouse trespass) need a mover and are not ported.
    pub fn test_collide_adjacent(&self, meta: &IsoMetaGrid, x: i32, y: i32, z: i32) -> bool {
        if self.collide_matrix == -1 {
            return true;
        }
        if !(-1..=1).contains(&x) || !(-1..=1).contains(&y) || !(-1..=1).contains(&z) {
            return true;
        }
        if !is_valid_chunk(meta, (self.x + x) / 8, (self.y + y) / 8) {
            return true;
        }
        get_matrix_bit(self.collide_matrix, x + 1, y + 1, z + 1)
    }

    // Edge elements.

    /// `isWall(GridSquareEdgeFacingDirection)`.
    pub fn is_wall_facing(&self, facing: EdgeFacing, m: &IsoSpriteManager) -> bool {
        let f = &m.tables.flag;
        let collide = match facing {
            EdgeFacing::NorthSouth => self.has(f.collideN),
            EdgeFacing::EastWest => self.has(f.collideW),
        };
        collide && !self.is_window_facing(facing, m)
    }

    /// `isWindow(GridSquareEdgeFacingDirection)`: the `windowN`/`windowW`
    /// flag.
    pub fn is_window_facing(&self, facing: EdgeFacing, m: &IsoSpriteManager) -> bool {
        let f = &m.tables.flag;
        match facing {
            EdgeFacing::NorthSouth => self.has(f.windowN),
            EdgeFacing::EastWest => self.has(f.windowW),
        }
    }

    /// `isDoor(GridSquareEdgeFacingDirection)`: the `doorN`/`doorW` flag.
    pub fn is_door_facing(&self, facing: EdgeFacing, m: &IsoSpriteManager) -> bool {
        let f = &m.tables.flag;
        match facing {
            EdgeFacing::NorthSouth => self.has(f.doorN),
            EdgeFacing::EastWest => self.has(f.doorW),
        }
    }

    /// `hasBlockedWindow(facing)`: an `IsoWindow` in `objects` on that edge
    /// with `isBlocked()` (closed and intact, or barricaded).
    pub fn has_blocked_window(&self, facing: EdgeFacing, hooks: &dyn SquareHooks) -> bool {
        self.objects.iter().any(|o| match &o.kind {
            ObjectKind::Window(w) => {
                EdgeFacing::of_north(w.north) == facing
                    && (!w.destroyed && !w.open || hooks.is_barricaded(self, o))
            }
            _ => false,
        })
    }

    /// `hasBlockedDoor(facing)`: an `IsoDoor` on that edge that is closed or
    /// barricaded, or a blocked thumpable door ([`SquareHooks`]).
    pub fn has_blocked_door(&self, facing: EdgeFacing, hooks: &dyn SquareHooks) -> bool {
        self.objects.iter().any(|o| match &o.kind {
            ObjectKind::Door(d) => {
                EdgeFacing::of_north(d.north) == facing && (!d.open || hooks.is_barricaded(self, o))
            }
            _ => false,
        }) || hooks.has_blocked_thumpable_door(self, facing)
    }

    /// `getWindow(GridSquareEdgeFacingDirection)`: the first `IsoWindow` in
    /// `specialObjects` on that edge.
    pub fn get_window_facing(&self, facing: EdgeFacing) -> Option<&IsoObject> {
        self.window_index(facing == EdgeFacing::NorthSouth)
            .map(|i| &self.objects[i])
    }

    /// `isEdgeElement(alongEdge, facingGetter, getSquare)`.
    pub fn is_edge_element(
        &self,
        edge: GridSquareEdge,
        g: &impl GetSquare,
        test: &dyn Fn(&IsoGridSquare, EdgeFacing) -> bool,
    ) -> bool {
        match edge {
            GridSquareEdge::North => test(self, EdgeFacing::NorthSouth),
            GridSquareEdge::West => test(self, EdgeFacing::EastWest),
            GridSquareEdge::South => g
                .get_grid_square(self.x, self.y + 1, self.z)
                .is_some_and(|s| test(s, EdgeFacing::NorthSouth)),
            GridSquareEdge::East => g
                .get_grid_square(self.x + 1, self.y, self.z)
                .is_some_and(|s| test(s, EdgeFacing::EastWest)),
        }
    }

    /// `getEdgeElement(alongEdge, facingGetter, getSquare)`.
    pub fn get_edge_element<'a, T>(
        &'a self,
        edge: GridSquareEdge,
        g: &'a impl GetSquare,
        get: &dyn Fn(&'a IsoGridSquare, EdgeFacing) -> Option<T>,
    ) -> Option<T> {
        match edge {
            GridSquareEdge::North => get(self, EdgeFacing::NorthSouth),
            GridSquareEdge::West => get(self, EdgeFacing::EastWest),
            GridSquareEdge::South => g
                .get_grid_square(self.x, self.y + 1, self.z)
                .and_then(|s| get(s, EdgeFacing::NorthSouth)),
            GridSquareEdge::East => g
                .get_grid_square(self.x + 1, self.y, self.z)
                .and_then(|s| get(s, EdgeFacing::EastWest)),
        }
    }

    /// `isEdgeElementTo(other, facingGetter, getSquare)`.
    pub fn is_edge_element_to(
        &self,
        other: Option<&IsoGridSquare>,
        g: &impl GetSquare,
        test: &dyn Fn(&IsoGridSquare, EdgeFacing) -> bool,
    ) -> bool {
        use GridSquareEdge::*;
        let Some(o) = other else { return false };
        if is(self, o) {
            return false;
        }
        let e = |s: &IsoGridSquare, edge| s.is_edge_element(edge, g, test);
        self.x > o.x && (e(self, West) || e(o, East))
            || self.x < o.x && (e(o, West) || e(self, East))
            || self.y > o.y && (e(self, North) || e(o, South))
            || self.y < o.y && (e(o, North) || e(self, South))
    }

    /// `getEdgeElementTo(next, edgeGetter, getSquare)`.
    pub fn get_edge_element_to<'a, T>(
        &'a self,
        next: Option<&'a IsoGridSquare>,
        g: &'a impl GetSquare,
        get: &dyn Fn(&'a IsoGridSquare, EdgeFacing) -> Option<T>,
    ) -> Option<T> {
        use GridSquareEdge::*;
        let o = next?;
        if is(self, o) {
            return None;
        }
        let e = |s: &'a IsoGridSquare, edge| s.get_edge_element(edge, g, get);
        if self.x > o.x {
            if let Some(v) = e(self, West).or_else(|| e(o, East)) {
                return Some(v);
            }
        }
        if self.x < o.x {
            if let Some(v) = e(o, West).or_else(|| e(self, East)) {
                return Some(v);
            }
        }
        if self.y < o.y {
            if let Some(v) = e(o, North).or_else(|| e(self, South)) {
                return Some(v);
            }
        }
        if self.y > o.y {
            if let Some(v) = e(self, North).or_else(|| e(o, South)) {
                return Some(v);
            }
        }
        None
    }

    /// `isWallTo(other, getSquare)`.
    pub fn is_wall_to(
        &self,
        other: Option<&IsoGridSquare>,
        m: &IsoSpriteManager,
        g: &impl GetSquare,
    ) -> bool {
        self.is_edge_element_to(other, g, &|s, f| s.is_wall_facing(f, m))
    }

    /// `isWindowTo(other, getSquare)`.
    pub fn is_window_to(
        &self,
        other: Option<&IsoGridSquare>,
        m: &IsoSpriteManager,
        g: &impl GetSquare,
    ) -> bool {
        self.is_edge_element_to(other, g, &|s, f| s.is_window_facing(f, m))
    }

    /// `isDoorTo(other, getSquare)`.
    pub fn is_door_to(
        &self,
        other: Option<&IsoGridSquare>,
        m: &IsoSpriteManager,
        g: &impl GetSquare,
    ) -> bool {
        self.is_edge_element_to(other, g, &|s, f| s.is_door_facing(f, m))
    }

    /// `isWindowBlockedTo(other, getSquare)`.
    pub fn is_window_blocked_to(
        &self,
        other: Option<&IsoGridSquare>,
        g: &impl GetSquare,
        hooks: &dyn SquareHooks,
    ) -> bool {
        self.is_edge_element_to(other, g, &|s, f| s.has_blocked_window(f, hooks))
    }

    /// `isDoorBlockedTo(other, getSquare)`.
    pub fn is_door_blocked_to(
        &self,
        other: Option<&IsoGridSquare>,
        g: &impl GetSquare,
        hooks: &dyn SquareHooks,
    ) -> bool {
        self.is_edge_element_to(other, g, &|s, f| s.has_blocked_door(f, hooks))
    }

    /// `getWindowTo(next, getSquare)`: the `IsoWindow` between this square
    /// and `next`.
    pub fn get_window_to<'a>(
        &'a self,
        next: Option<&'a IsoGridSquare>,
        g: &'a impl GetSquare,
    ) -> Option<&'a IsoObject> {
        self.get_edge_element_to(next, g, &|s: &'a IsoGridSquare, f| s.get_window_facing(f))
    }
}

/// `IsoMetaGrid.isValidChunk(wx, wy)`.
fn is_valid_chunk(meta: &IsoMetaGrid, wx: i32, wy: i32) -> bool {
    let (x, y) = (wx * 8, wy * 8);
    x >= meta.min_x * 256
        && x < (meta.max_x + 1) * 256
        && y >= meta.min_y * 256
        && y < (meta.max_y + 1) * 256
}

/// One `ReCalculateCollide`/`ReCalculatePathFind`/`ReCalculateVisionBlocked`
/// result: the bits square `at` stores for neighbour `to`.
struct MatrixBits {
    at: (i32, i32, i32),
    to: (i32, i32, i32),
    collide: bool,
    path: bool,
    vision: bool,
}

fn matrix_bits(
    a: &IsoGridSquare,
    b: &IsoGridSquare,
    m: &IsoSpriteManager,
    g: &impl GetSquare,
) -> MatrixBits {
    MatrixBits {
        at: (a.x, a.y, a.z),
        to: (b.x, b.y, b.z),
        collide: a.calculate_collide(Some(b), false, false, false, false, m, g),
        path: a.calculate_collide(Some(b), false, true, false, false, m, g),
        vision: a.calculate_vision_blocked(Some(b), m, g),
    }
}

/// `ReCalculateAll(bDoReverse, a, getter)`'s matrix updates for the pair.
fn recalc_pair(
    sq: &IsoGridSquare,
    a: &IsoGridSquare,
    reverse: bool,
    m: &IsoSpriteManager,
    g: &impl GetSquare,
    out: &mut Vec<MatrixBits>,
) {
    if is(sq, a) {
        return;
    }
    out.push(matrix_bits(sq, a, m, g));
    if reverse {
        out.push(matrix_bits(a, sq, m, g));
    }
}

/// `RecalcAllWithNeighbours(bDoReverse, getter)`'s matrix updates (the
/// neighbourhood, then the four corner pairs through `nav`).
fn recalc_all_with_neighbours(
    sq: &IsoGridSquare,
    meta: &IsoMetaGrid,
    m: &IsoSpriteManager,
    g: &impl GetSquare,
    out: &mut Vec<MatrixBits>,
) {
    for x in sq.x - 1..=sq.x + 1 {
        for y in sq.y - 1..=sq.y + 1 {
            for z in sq.z - 1..=sq.z + 1 {
                if !(MIN_LEVEL..=MAX_LEVEL).contains(&z) || !meta.is_valid_square(x, y) {
                    continue;
                }
                if (x, y, z) == (sq.x, sq.y, sq.z) {
                    continue;
                }
                if let Some(n) = g.get_grid_square(x, y, z) {
                    recalc_pair(sq, n, true, m, g, out);
                }
            }
        }
    }
    let nav = |d: IsoDirections| sq.get_adjacent_square(d, g);
    let (n, s, w, e) = (
        nav(IsoDirections::N),
        nav(IsoDirections::S),
        nav(IsoDirections::W),
        nav(IsoDirections::E),
    );
    for (a, b) in [(n, w), (n, e), (s, w), (s, e)] {
        if let (Some(a), Some(b)) = (a, b) {
            recalc_pair(a, b, true, m, g, out);
        }
    }
}

impl IsoCell {
    fn apply_matrix_bits(&mut self, bits: Vec<MatrixBits>) {
        for b in bits {
            let Some(sq) = self.get_grid_square_mut(b.at.0, b.at.1, b.at.2) else {
                continue;
            };
            let (dx, dy, dz) = (
                1 + b.to.0 - b.at.0,
                1 + b.to.1 - b.at.1,
                1 + b.to.2 - b.at.2,
            );
            sq.collide_matrix = set_matrix_bit(sq.collide_matrix, dx, dy, dz, b.collide);
            sq.path_matrix = set_matrix_bit(sq.path_matrix, dx, dy, dz, b.path);
            sq.vision_matrix = set_matrix_bit(sq.vision_matrix, dx, dy, dz, b.vision);
        }
    }

    /// The square part of `IsoChunk.loadInWorldStreamerThread()` for chunk
    /// `(wx, wy)`, followed by the chunk-edge pass of `doLoadGridsquare`
    /// against already loaded neighbour chunks:
    ///
    /// 1. creates the missing level-0 squares and the 3x3 neighbourhood of
    ///    every non-empty square on its level and the levels below
    ///    (`ensureNotNull3x3`, rooms from the meta grid);
    /// 2. `RecalcProperties` for every square;
    /// 3. `haveRoof` below the topmost rain-blocking square of each column;
    /// 4. `RecalcAllWithNeighbours(true, chunkGetter)` for every square;
    /// 5. for squares on the chunk edge, the collide/path/vision bits with
    ///    the neighbours across the edge, through the cell
    ///    (`RecalcAllWithNeighbour`).
    ///
    /// `doLoadGridsquare`'s `EnsureSurroundNotNull` across chunk edges is not
    /// ported.
    pub fn recalc_chunk_squares(
        &mut self,
        wx: i32,
        wy: i32,
        meta: &IsoMetaGrid,
        m: &IsoSpriteManager,
    ) {
        let Some(ch) = self.chunks.get_mut(&(wx, wy)) else {
            return;
        };
        ensure_squares(ch, meta);
        let (min, max) = (ch.min_level, ch.max_level);
        for z in min..=max {
            let mut any = false;
            for y in 0..8 {
                for x in 0..8 {
                    if let Some(sq) = ch.get_grid_square_mut(x, y, z) {
                        sq.recalc_properties(m);
                        any = true;
                    }
                }
            }
            // `RecalcProperties` ends with `chunk.checkPhysicsLater(z)`.
            if any {
                ch.check_physics_later(z);
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                let mut z2 = max;
                while z2 > 0 {
                    if ch
                        .get_grid_square(x, y, z2)
                        .is_some_and(|s| s.has_rain_blocking_tile(m))
                    {
                        z2 -= 1;
                        while z2 >= min {
                            if let Some(sq) = ch.get_grid_square_mut(x, y, z2) {
                                if !sq.have_roof {
                                    sq.have_roof = true;
                                    sq.properties.unset_flag(m.tables.flag.exterior);
                                }
                            }
                            z2 -= 1;
                        }
                        break;
                    }
                    z2 -= 1;
                }
            }
        }
        let mut bits = Vec::new();
        {
            let ch = &self.chunks[&(wx, wy)];
            let g = ChunkGetter(ch);
            for z in min..=max {
                for y in 0..8 {
                    for x in 0..8 {
                        if let Some(sq) = ch.get_grid_square(x, y, z) {
                            recalc_all_with_neighbours(sq, meta, m, &g, &mut bits);
                        }
                    }
                }
            }
        }
        // `recalc_all_with_neighbours` applies results as it goes in Java;
        // the bits only depend on properties, so applying them afterwards
        // gives the same matrices.
        self.apply_matrix_bits(bits);
        let mut bits = Vec::new();
        {
            let ch = &self.chunks[&(wx, wy)];
            for z in min..=max {
                for y in 0..8 {
                    for x in 0..8 {
                        let Some(sq) = ch.get_grid_square(x, y, z) else {
                            continue;
                        };
                        if !(x == 0 || x == 7 || y == 0 || y == 7) {
                            continue;
                        }
                        for dz in -1..=1 {
                            for dir in IsoDirections::ALL {
                                let (dx, dy) = dir.delta();
                                let (nx, ny) = (x + dx, y + dy);
                                if (0..8).contains(&nx) && (0..8).contains(&ny) {
                                    continue;
                                }
                                if let Some(n) =
                                    self.get_grid_square(sq.x + dx, sq.y + dy, sq.z + dz)
                                {
                                    recalc_pair(sq, n, true, m, self, &mut bits);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.apply_matrix_bits(bits);
    }

    /// `RecalcAllWithNeighbours(true)` through the cell, after a square's
    /// objects changed (its properties are recalculated first).
    pub fn recalc_all_with_neighbours(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        meta: &IsoMetaGrid,
        m: &IsoSpriteManager,
    ) {
        let Some(sq) = self.get_grid_square_mut(x, y, z) else {
            return;
        };
        sq.recalc_properties(m);
        if let Some(ch) = self.chunks.get_mut(&(x.div_euclid(8), y.div_euclid(8))) {
            ch.check_physics_later(z);
        }
        let mut bits = Vec::new();
        if let Some(sq) = self.get_grid_square(x, y, z) {
            recalc_all_with_neighbours(sq, meta, m, self, &mut bits);
        }
        self.apply_matrix_bits(bits);
    }

    /// `getGridSquare(x, y, z)`, mutable.
    pub fn get_grid_square_mut(&mut self, x: i32, y: i32, z: i32) -> Option<&mut IsoGridSquare> {
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        self.chunks
            .get_mut(&(wx, wy))?
            .get_grid_square_mut(x - wx * 8, y - wy * 8, z)
    }
}

/// `loadInWorldStreamerThread`'s square creation: a level-0 square
/// everywhere, and `ensureNotNull3x3` around every square with objects on
/// its level and every level below down to `minLevel + 1`.
fn ensure_squares(ch: &mut IsoChunk, meta: &IsoMetaGrid) {
    let (wx, wy) = (ch.wx, ch.wy);
    // `IsoGridSquare.getNew` + `IsoChunk.setSquare` (which sets
    // `associatedBuilding`).
    let new_square = |lx: i32, ly: i32, z: i32| {
        let (x, y) = (wx * 8 + lx, wy * 8 + ly);
        let mut sq = IsoGridSquare::new(x, y, z);
        sq.associated_building = meta.get_associated_building_at(x, y).ok().flatten();
        sq
    };
    // `ensureNotNull`: also `assignRoom` (the room lookup's `Rand` draw for
    // a new `IsoBuilding` is not made here).
    let ensure = |ch: &mut IsoChunk, lx: i32, ly: i32, z: i32| {
        if !(0..8).contains(&lx) || !(0..8).contains(&ly) || ch.get_grid_square(lx, ly, z).is_some()
        {
            return;
        }
        let mut sq = new_square(lx, ly, z);
        sq.room_id = meta.get_room_at(sq.x, sq.y, z).map_or(-1, |r| r.id);
        ch.set_square(lx, ly, z, Some(sq));
    };
    let ensure3x3 = |ch: &mut IsoChunk, lx: i32, ly: i32, z: i32| {
        ensure(ch, lx, ly, z);
        for d in IsoDirections::ALL {
            let (dx, dy) = d.delta();
            ensure(ch, lx + dx, ly + dy, z);
        }
    };
    let mut z = ch.min_level;
    while z <= ch.max_level {
        for y in 0..8 {
            for x in 0..8 {
                if ch.get_grid_square(x, y, z).is_none() && z == 0 {
                    ch.set_square(x, y, z, Some(new_square(x, y, z)));
                }
                let Some(sq) = ch.get_grid_square(x, y, z) else {
                    continue;
                };
                if !sq.objects.is_empty() {
                    ensure3x3(ch, x, y, z);
                    let mut zz = z - 1;
                    while zz > ch.min_level {
                        ensure3x3(ch, x, y, zz);
                        zz -= 1;
                    }
                }
            }
        }
        z += 1;
    }
}

#[cfg(test)]
mod tests;
