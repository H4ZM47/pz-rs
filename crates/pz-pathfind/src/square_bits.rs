//! What the native pathfinder knows about each square:
//! `zombie.pathfind.nativeCode.SquareUpdateTask.getBits/getCost` and the
//! buffers `ChunkUpdateTask.init(IsoChunk)` and `SquareUpdateTask.init`
//! hand to `libPZPathFind` (`PathfindNative.updateChunk/updateSquare`).
//!
//! Every query reads the world model of `pz-world` (recalculated square
//! properties, objects, rooms, neighbours through a [`GetSquare`]). Flag and
//! object-type ordinals come from the runtime [`SpriteTables`]; the ones
//! `pz-world` does not name are resolved by name in [`BitsTables`].
//!
//! State `pz-world` does not model yet goes through [`SquareBitsHooks`]; its
//! defaults are a fresh single-player world:
//!
//! * barricades: none (`isBarricaded()` is false for windows, window frames
//!   and thumpables); [`pz_world::ObjectHooks`] implements
//!   [`SquareBitsHooks`] with the barricades of the loaded `IsoBarricade`
//!   objects, as the game does, and every other default;
//! * `hasLitCampfire()`: false (no campfire global objects);
//! * `hasFarmingPlant()`: false (no farming global objects);
//! * `BentFences.isEnabled()`: false (the default sandbox disables bent
//!   fences), so `isUnbentObject` is never asked;
//! * `GameClient.client`: false, so `SafeHouse.getSafeHouse` is never asked.
//!
//! Sheet ropes are read from the square flags (`climbSheet*`), as the
//! original does, so need no hook.
//!
//! Parity: the `square_bits_replay` example loads real B42 map chunks with
//! `pz-world`'s lotpack loader and compares the chunk buffers byte for byte
//! with the ones the game's own `ChunkUpdateTask.init` produced for the same
//! chunks (612 chunks in five regions). Every square whose inputs match the
//! game's is bit-exact (99926 of 101056 squares, 512 of 612 chunk buffers
//! byte-identical); the rest have different objects because `pz-world` does
//! not run B42 world generation (biome vegetation) yet. Details are in that
//! example's module doc.

use pz_world::chunk::{IsoChunk, IsoGridSquare};
use pz_world::objects::{IsoObject, IsoThumpable, ObjectKind};
use pz_world::sprite::{
    IsoFlagType, IsoObjectType, IsoSpriteManager, PropertyContainer, SpriteTables,
};
use pz_world::{GetSquare, IsoDirections, IsoMetaGrid};

/// `SquareUpdateTask.BIT_*`: the bits of a square's `long` in the native
/// pathfinder's input.
pub mod bits {
    pub const SOLID: u64 = 1;
    pub const COLLIDE_W: u64 = 1 << 1;
    pub const COLLIDE_N: u64 = 1 << 2;
    pub const STAIR_TW: u64 = 1 << 3;
    pub const STAIR_MW: u64 = 1 << 4;
    pub const STAIR_BW: u64 = 1 << 5;
    pub const STAIR_TN: u64 = 1 << 6;
    pub const STAIR_MN: u64 = 1 << 7;
    pub const STAIR_BN: u64 = 1 << 8;
    pub const SOLID_FLOOR: u64 = 1 << 9;
    pub const SOLID_TRANS: u64 = 1 << 10;
    pub const WINDOW_W: u64 = 1 << 11;
    pub const WINDOW_N: u64 = 1 << 12;
    pub const CAN_PATH_W: u64 = 1 << 13;
    pub const CAN_PATH_N: u64 = 1 << 14;
    pub const THUMP_W: u64 = 1 << 15;
    pub const THUMP_N: u64 = 1 << 16;
    pub const THUMPABLE: u64 = 1 << 17;
    pub const DOOR_E: u64 = 1 << 18;
    pub const DOOR_S: u64 = 1 << 19;
    pub const WINDOW_W_UNBLOCKED: u64 = 1 << 20;
    pub const WINDOW_N_UNBLOCKED: u64 = 1 << 21;
    pub const DOOR_W_UNBLOCKED: u64 = 1 << 22;
    pub const DOOR_N_UNBLOCKED: u64 = 1 << 23;
    pub const HOPPABLE_N: u64 = 1 << 24;
    pub const HOPPABLE_W: u64 = 1 << 25;
    pub const TARGET_PATH_W: u64 = 1 << 26;
    pub const TARGET_PATH_N: u64 = 1 << 27;
    pub const BENDABLE_W: u64 = 1 << 28;
    pub const BENDABLE_N: u64 = 1 << 29;
    pub const TALL_HOPPABLE_N: u64 = 1 << 30;
    pub const TALL_HOPPABLE_W: u64 = 1 << 31;
    pub const FARMING_PLANT: u64 = 1 << 32;
    pub const ALL_SOLID_BITS: u64 = SOLID | SOLID_TRANS;
    pub const ALL_STAIR_BITS: u64 = STAIR_TW | STAIR_MW | STAIR_BW | STAIR_TN | STAIR_MN | STAIR_BN;
}

/// The `slopedSurfaceDirection` value `updateSquare` receives for a square
/// without a sloped surface (one past `IsoDirections.NE`).
pub const NO_SLOPE_DIRECTION: i32 = 8;

/// Game state `getBits`/`getCost` read that `pz-world` does not model yet.
/// Every default is a fresh single-player world (see the module doc).
pub trait SquareBitsHooks {
    /// `isBarricaded()` of an `IsoWindow`, `IsoWindowFrame` or
    /// `IsoThumpable` on `sq`.
    fn is_barricaded(&self, _sq: &IsoGridSquare, _obj: &IsoObject) -> bool {
        false
    }

    /// `IsoGridSquare.hasLitCampfire()` (the `campfire` global object
    /// system).
    fn has_lit_campfire(&self, _sq: &IsoGridSquare) -> bool {
        false
    }

    /// `IsoGridSquare.hasFarmingPlant()` (the `farming` global object
    /// system).
    fn has_farming_plant(&self, _sq: &IsoGridSquare) -> bool {
        false
    }

    /// `BentFences.getInstance().isEnabled()`.
    fn bent_fences_enabled(&self) -> bool {
        false
    }

    /// `BentFences.getInstance().isUnbentObject(obj)`.
    fn is_unbent_object(&self, _sq: &IsoGridSquare, _obj: &IsoObject) -> bool {
        false
    }

    /// `GameClient.client`.
    fn is_client(&self) -> bool {
        false
    }

    /// `SafeHouse.getSafeHouse(sq) != null`.
    fn in_safehouse(&self, _sq: &IsoGridSquare) -> bool {
        false
    }
}

/// [`SquareBitsHooks`] with every default.
pub struct DefaultBitsHooks;

impl SquareBitsHooks for DefaultBitsHooks {}

/// The loaded objects' barricades (`IsoBarricade` on the object's square or
/// the square across its edge); the other hooks keep their defaults.
impl SquareBitsHooks for pz_world::ObjectHooks<'_> {
    fn is_barricaded(&self, sq: &IsoGridSquare, obj: &IsoObject) -> bool {
        self.object_barricaded(sq, obj)
    }
}

/// The `IsoFlagType` / `IsoObjectType` constants these queries use,
/// resolved by name from the runtime tables.
#[allow(non_snake_case)]
#[derive(Clone, Copy, Debug)]
pub struct BitsTables {
    pub solidfloor: IsoFlagType,
    pub solid: IsoFlagType,
    pub solidtrans: IsoFlagType,
    pub collideW: IsoFlagType,
    pub collideN: IsoFlagType,
    pub HoppableN: IsoFlagType,
    pub HoppableW: IsoFlagType,
    pub TallHoppableN: IsoFlagType,
    pub TallHoppableW: IsoFlagType,
    pub WallN: IsoFlagType,
    pub WallW: IsoFlagType,
    pub WallNTrans: IsoFlagType,
    pub WallWTrans: IsoFlagType,
    pub windowW: IsoFlagType,
    pub windowN: IsoFlagType,
    pub WindowW: IsoFlagType,
    pub WindowN: IsoFlagType,
    pub canPathW: IsoFlagType,
    pub canPathN: IsoFlagType,
    pub DoorWallW: IsoFlagType,
    pub DoorWallN: IsoFlagType,
    pub water: IsoFlagType,
    pub CantClimb: IsoFlagType,
    pub isEave: IsoFlagType,
    pub cutN: IsoFlagType,
    pub cutW: IsoFlagType,
    pub makeWindowInvincible: IsoFlagType,
    pub attachedE: IsoFlagType,
    pub attachedS: IsoFlagType,
    /// `climbSheetTopW/N/E/S`, `climbSheetW/N/E/S` (`IsoWindow.isSheetRopeHere`).
    pub climb_sheet: [IsoFlagType; 8],
    pub stairsTW: IsoObjectType,
    pub stairsMW: IsoObjectType,
    pub stairsBW: IsoObjectType,
    pub stairsTN: IsoObjectType,
    pub stairsMN: IsoObjectType,
    pub stairsBN: IsoObjectType,
    pub WestRoofB: IsoObjectType,
    pub WestRoofM: IsoObjectType,
    pub WestRoofT: IsoObjectType,
}

impl BitsTables {
    pub fn resolve(t: &SpriteTables) -> Result<Self, String> {
        let f = |n: &str| {
            t.flag_from_string(n)
                .ok_or_else(|| format!("IsoFlagType has no {n}"))
        };
        let o = |n: &str| {
            let ty = t.object_type_from_string(n);
            if ty == t.object_type_max() {
                Err(format!("IsoObjectType has no {n}"))
            } else {
                Ok(ty)
            }
        };
        Ok(BitsTables {
            solidfloor: f("solidfloor")?,
            solid: f("solid")?,
            solidtrans: f("solidtrans")?,
            collideW: f("collideW")?,
            collideN: f("collideN")?,
            HoppableN: f("HoppableN")?,
            HoppableW: f("HoppableW")?,
            TallHoppableN: f("TallHoppableN")?,
            TallHoppableW: f("TallHoppableW")?,
            WallN: f("WallN")?,
            WallW: f("WallW")?,
            WallNTrans: f("WallNTrans")?,
            WallWTrans: f("WallWTrans")?,
            windowW: f("windowW")?,
            windowN: f("windowN")?,
            WindowW: f("WindowW")?,
            WindowN: f("WindowN")?,
            canPathW: f("canPathW")?,
            canPathN: f("canPathN")?,
            DoorWallW: f("DoorWallW")?,
            DoorWallN: f("DoorWallN")?,
            water: f("water")?,
            CantClimb: f("CantClimb")?,
            isEave: f("isEave")?,
            cutN: f("cutN")?,
            cutW: f("cutW")?,
            makeWindowInvincible: f("makeWindowInvincible")?,
            attachedE: f("attachedE")?,
            attachedS: f("attachedS")?,
            climb_sheet: [
                f("climbSheetTopW")?,
                f("climbSheetTopN")?,
                f("climbSheetTopE")?,
                f("climbSheetTopS")?,
                f("climbSheetW")?,
                f("climbSheetN")?,
                f("climbSheetE")?,
                f("climbSheetS")?,
            ],
            stairsTW: o("stairsTW")?,
            stairsMW: o("stairsMW")?,
            stairsBW: o("stairsBW")?,
            stairsTN: o("stairsTN")?,
            stairsMN: o("stairsMN")?,
            stairsBN: o("stairsBN")?,
            WestRoofB: o("WestRoofB")?,
            WestRoofM: o("WestRoofM")?,
            WestRoofT: o("WestRoofT")?,
        })
    }
}

/// A square's sloped surface as the native pathfinder receives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlopedSurface {
    pub direction: IsoDirections,
    /// `getSlopedSurfaceHeightMin()`.
    pub min: f32,
    /// `getSlopedSurfaceHeightMax()`.
    pub max: f32,
}

/// The arguments `SquareUpdateTask.execute` passes to
/// `PathfindNative.updateSquare` (after `SquareUpdateTask.init(square)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquareUpdate {
    pub x: i32,
    pub y: i32,
    /// `square.z + 32`.
    pub z: i32,
    pub bits: u64,
    pub cost: i16,
    /// `IsoDirections.ordinal()`, or [`NO_SLOPE_DIRECTION`].
    pub slope_direction: i32,
    pub slope_min: f32,
    pub slope_max: f32,
}

/// The world `getBits`/`getCost` read: the sprites, the meta grid (rooms
/// and buildings), the squares around (`IsoWorld.instance.currentCell`) and
/// the hooks for state `pz-world` does not model.
pub struct SquareBits<'a, G: GetSquare> {
    pub sprites: &'a IsoSpriteManager,
    pub meta: &'a IsoMetaGrid,
    pub squares: &'a G,
    pub hooks: &'a dyn SquareBitsHooks,
    pub t: BitsTables,
}

/// `IsoObject.getProperties()`: the sprite's properties.
fn props<'m>(m: &'m IsoSpriteManager, o: &IsoObject) -> Option<&'m PropertyContainer> {
    o.sprite.map(|s| &m.sprites[s].properties)
}

/// `square.getSpecialObjects()`, skipping identities no longer in
/// `objects`.
fn specials(sq: &IsoGridSquare) -> impl Iterator<Item = &IsoObject> {
    sq.special_objects
        .iter()
        .filter_map(move |&id| sq.objects.iter().find(|o| o.id == id))
}

impl<'a, G: GetSquare> SquareBits<'a, G> {
    pub fn new(
        sprites: &'a IsoSpriteManager,
        meta: &'a IsoMetaGrid,
        squares: &'a G,
        hooks: &'a dyn SquareBitsHooks,
    ) -> Result<Self, String> {
        Ok(SquareBits {
            sprites,
            meta,
            squares,
            hooks,
            t: BitsTables::resolve(&sprites.tables)?,
        })
    }

    fn obj_has(&self, o: &IsoObject, f: IsoFlagType) -> bool {
        props(self.sprites, o).is_some_and(|p| p.has(f))
    }

    /// `SquareUpdateTask.getBits(square)`.
    pub fn get_bits(&self, sq: &IsoGridSquare) -> u64 {
        use bits::*;
        let t = &self.t;
        let mut b = 0u64;
        if sq.has(t.solidfloor) {
            b |= SOLID_FLOOR;
        }
        if sq.has(t.solid) {
            b |= SOLID;
        }
        if sq.has(t.solidtrans) {
            b |= SOLID_TRANS;
        }
        if sq.has(t.collideW) {
            b |= COLLIDE_W;
        }
        if sq.has(t.collideN) {
            b |= COLLIDE_N;
        }
        for (ty, bit) in [
            (t.stairsTW, STAIR_TW),
            (t.stairsMW, STAIR_MW),
            (t.stairsBW, STAIR_BW),
            (t.stairsTN, STAIR_TN),
            (t.stairsMN, STAIR_MN),
            (t.stairsBN, STAIR_BN),
        ] {
            if sq.has_type(ty) {
                b |= bit;
            }
        }
        if sq.has(t.HoppableN) {
            b |= HOPPABLE_N;
        } else if (sq.has(t.TallHoppableN) || sq.has(t.WallN) || sq.has(t.WallNTrans))
            && self.can_climb_over_wall(Some(sq), IsoDirections::N)
            && self.can_climb_over_wall(Some(sq), IsoDirections::S)
        {
            b |= TALL_HOPPABLE_N;
        }
        if sq.has(t.HoppableW) {
            b |= HOPPABLE_W;
        } else if (sq.has(t.TallHoppableW) || sq.has(t.WallW) || sq.has(t.WallWTrans))
            && self.can_climb_over_wall(Some(sq), IsoDirections::W)
            && self.can_climb_over_wall(Some(sq), IsoDirections::E)
        {
            b |= TALL_HOPPABLE_W;
        }
        if sq.has(t.windowW) || sq.has(t.WindowW) {
            b |= WINDOW_W | COLLIDE_W;
            if self.is_window_unblocked(sq, false) {
                b |= WINDOW_W_UNBLOCKED;
            }
        }
        if sq.has(t.windowN) || sq.has(t.WindowN) {
            b |= WINDOW_N | COLLIDE_N;
            if self.is_window_unblocked(sq, true) {
                b |= WINDOW_N_UNBLOCKED;
            }
        }
        if sq.has(t.canPathW) {
            b |= CAN_PATH_W;
        }
        if sq.has(t.canPathN) {
            b |= CAN_PATH_N;
        }
        let mut has_door_w = false;
        let mut has_door_n = false;
        for obj in specials(sq) {
            let mut dir = None;
            match &obj.kind {
                ObjectKind::Door(d) => {
                    dir = self.door_sprite_edge(obj, d.north, d.open, false);
                    if d.open {
                        match self.door_sprite_edge(obj, d.north, d.open, true) {
                            Some(IsoDirections::N) => b |= DOOR_N_UNBLOCKED,
                            Some(IsoDirections::W) => b |= DOOR_W_UNBLOCKED,
                            _ => {}
                        }
                        dir = None;
                    }
                }
                ObjectKind::Thumpable(th) if th.is_door => {
                    dir = self.thumpable_sprite_edge(obj, th, false);
                    if th.open {
                        match self.thumpable_sprite_edge(obj, th, true) {
                            Some(IsoDirections::N) => b |= DOOR_N_UNBLOCKED,
                            Some(IsoDirections::W) => b |= DOOR_W_UNBLOCKED,
                            _ => {}
                        }
                        dir = None;
                    }
                }
                _ => {}
            }
            match dir {
                Some(IsoDirections::W) => {
                    b |= CAN_PATH_W | COLLIDE_W;
                    has_door_w = true;
                }
                Some(IsoDirections::N) => {
                    b |= CAN_PATH_N | COLLIDE_N;
                    has_door_n = true;
                }
                Some(IsoDirections::S) => b |= DOOR_S,
                Some(IsoDirections::E) => b |= DOOR_E,
                _ => {}
            }
        }
        if sq.has(t.DoorWallW) {
            b |= CAN_PATH_W | COLLIDE_W;
            if !has_door_w {
                b |= DOOR_W_UNBLOCKED;
            }
        }
        if sq.has(t.DoorWallN) {
            b |= CAN_PATH_N | COLLIDE_N;
            if !has_door_n {
                b |= DOOR_N_UNBLOCKED;
            }
        }
        if self.has_square_thumpable(sq) {
            b |= CAN_PATH_W | CAN_PATH_N | THUMPABLE;
            if !sq.has(t.solid) && !sq.has(t.solidtrans) {
                b |= SOLID_TRANS;
            }
        }
        if self.hooks.has_lit_campfire(sq) {
            b |= CAN_PATH_W | CAN_PATH_N | THUMPABLE | SOLID_TRANS;
        }
        if self.has_wall_thumpable(sq, true) {
            b |= CAN_PATH_N | THUMP_N;
        }
        if self.has_wall_thumpable(sq, false) {
            b |= CAN_PATH_W | THUMP_W;
        }
        if self.hooks.bent_fences_enabled() {
            if self.has_wall_bendable(sq, true) {
                b |= CAN_PATH_N | THUMP_N | BENDABLE_N | TARGET_PATH_N;
            }
            if self.has_wall_bendable(sq, false) {
                b |= CAN_PATH_W | THUMP_W | BENDABLE_W | TARGET_PATH_W;
            }
        }
        if self.hooks.has_farming_plant(sq) {
            b |= FARMING_PLANT;
        }
        b
    }

    /// `SquareUpdateTask.getCost(square)`: 5 on a tree or bush.
    pub fn get_cost(&self, sq: &IsoGridSquare) -> i16 {
        let mut cost = 0i16;
        if sq.has_tree || self.has_bush(sq) {
            cost += 5;
        }
        cost
    }

    /// `IsoGridSquare.hasBush()`: an object whose sprite is in tileset
    /// `f_bushes_1` or has a `Bush` property (`IsoObject.isBush()`).
    pub fn has_bush(&self, sq: &IsoGridSquare) -> bool {
        sq.objects.iter().any(|o| {
            o.sprite.is_some_and(|s| {
                let sp = &self.sprites.sprites[s];
                sp.tileset_name.as_deref() == Some("f_bushes_1")
                    || sp.properties.get(&self.sprites.alias, "Bush").is_some()
            })
        })
    }

    /// The sloped surface of `sq`, if any.
    pub fn sloped_surface(&self, sq: &IsoGridSquare) -> Option<SlopedSurface> {
        sq.sloped_surface_direction.map(|direction| SlopedSurface {
            direction,
            min: slope_min(sq),
            max: slope_max(sq),
        })
    }

    /// `SquareUpdateTask.init(square)`: the `updateSquare` arguments.
    pub fn square_update(&self, sq: &IsoGridSquare) -> SquareUpdate {
        SquareUpdate {
            x: sq.x,
            y: sq.y,
            z: sq.z + 32,
            bits: self.get_bits(sq),
            cost: self.get_cost(sq),
            slope_direction: sq
                .sloped_surface_direction
                .map_or(NO_SLOPE_DIRECTION, |d| d as i32),
            slope_min: slope_min(sq),
            slope_max: slope_max(sq),
        }
    }

    /// `ChunkUpdateTask.init(chunk)`: the `updateChunk` buffer, big-endian:
    /// `minLevel+32`, `maxLevel+32` (i32), then for each level, row and
    /// column the square's bits (i64) and cost (i16), zeros for a missing
    /// square, then the number of sloped squares (i16) and for each
    /// `x, y, z+32, direction ordinal` (u8) and min/max height (f32).
    ///
    /// The original's direct buffer is rounded up to 256 bytes; the native
    /// side reads the self-delimiting content only, which is what this
    /// returns.
    pub fn chunk_update_buffer(&self, chunk: &IsoChunk) -> Vec<u8> {
        let mut bb = Vec::with_capacity(
            8 + (chunk.max_level - chunk.min_level + 1).max(0) as usize * 64 * 10 + 2,
        );
        let mut slopes = Vec::new();
        let mut num_sloped: i16 = 0;
        bb.extend_from_slice(&(chunk.min_level + 32).to_be_bytes());
        bb.extend_from_slice(&(chunk.max_level + 32).to_be_bytes());
        for z in chunk.min_level..=chunk.max_level {
            for y in 0..8 {
                for x in 0..8 {
                    let Some(sq) = chunk.get_grid_square(x, y, z) else {
                        bb.extend_from_slice(&0i64.to_be_bytes());
                        bb.extend_from_slice(&0i16.to_be_bytes());
                        continue;
                    };
                    bb.extend_from_slice(&self.get_bits(sq).to_be_bytes());
                    bb.extend_from_slice(&self.get_cost(sq).to_be_bytes());
                    let Some(dir) = sq.sloped_surface_direction else {
                        continue;
                    };
                    slopes.extend_from_slice(&[x as u8, y as u8, (z + 32) as u8, dir as u8]);
                    slopes.extend_from_slice(&slope_min(sq).to_be_bytes());
                    slopes.extend_from_slice(&slope_max(sq).to_be_bytes());
                    num_sloped = num_sloped.wrapping_add(1);
                }
            }
        }
        bb.extend_from_slice(&num_sloped.to_be_bytes());
        bb.extend_from_slice(&slopes);
        bb
    }

    /// `IsoDoor.getSpriteEdge(ignoreOpen)`.
    fn door_sprite_edge(
        &self,
        obj: &IsoObject,
        north: bool,
        open: bool,
        ignore_open: bool,
    ) -> Option<IsoDirections> {
        let closed_edge = if north {
            IsoDirections::N
        } else {
            IsoDirections::W
        };
        if !open || ignore_open {
            return Some(closed_edge);
        }
        let p = props(self.sprites, obj);
        if p.is_some_and(|p| p.has_name(&self.sprites.alias, "GarageDoor")) {
            return Some(closed_edge);
        }
        Some(self.open_edge(p, north))
    }

    /// `IsoThumpable.getSpriteEdge(ignoreOpen)`.
    fn thumpable_sprite_edge(
        &self,
        obj: &IsoObject,
        th: &IsoThumpable,
        ignore_open: bool,
    ) -> Option<IsoDirections> {
        if !th.is_door && !self.is_window_object(obj) {
            return None;
        }
        if !th.open || ignore_open {
            return Some(if th.north {
                IsoDirections::N
            } else {
                IsoDirections::W
            });
        }
        Some(self.open_edge(props(self.sprites, obj), th.north))
    }

    /// The open-door edge shared by `IsoDoor` and `IsoThumpable`.
    fn open_edge(&self, p: Option<&PropertyContainer>, north: bool) -> IsoDirections {
        if p.is_some_and(|p| p.has(self.t.attachedE)) {
            IsoDirections::E
        } else if p.is_some_and(|p| p.has(self.t.attachedS)) {
            IsoDirections::S
        } else if north {
            IsoDirections::W
        } else {
            IsoDirections::N
        }
    }

    /// `IsoObject.isWindow()`: the sprite has `WindowN` or `WindowW`.
    fn is_window_object(&self, o: &IsoObject) -> bool {
        self.obj_has(o, self.t.WindowN) || self.obj_has(o, self.t.WindowW)
    }

    /// `IsoObject.isWallN()` / `isWallW()`.
    fn is_wall_object(&self, o: &IsoObject, north: bool) -> bool {
        if north {
            self.obj_has(o, self.t.WallN) || self.obj_has(o, self.t.WallNTrans)
        } else {
            self.obj_has(o, self.t.WallW) || self.obj_has(o, self.t.WallWTrans)
        }
    }

    /// The closed sprite's properties when `IsoThumpable.isHoppable()` /
    /// `isTallHoppable()` read them (a closed door), else the sprite's.
    fn thumpable_hop_props(
        &self,
        obj: &IsoObject,
        th: &IsoThumpable,
    ) -> Option<&PropertyContainer> {
        if th.is_door && !th.open {
            if let Some(cs) = th.closed_sprite {
                return Some(&self.sprites.sprites[cs].properties);
            }
        }
        props(self.sprites, obj)
    }

    /// `IsoThumpable.isHoppable()`.
    fn thumpable_hoppable(&self, obj: &IsoObject, th: &IsoThumpable) -> bool {
        let closed_door = th.is_door && !th.open && th.closed_sprite.is_some();
        let p = self.thumpable_hop_props(obj, th);
        let hop = p.is_some_and(|p| p.has(self.t.HoppableN) || p.has(self.t.HoppableW));
        if closed_door {
            return hop;
        }
        hop || th.is_hoppable
    }

    /// `IsoThumpable.isTallHoppable()`.
    fn thumpable_tall_hoppable(&self, obj: &IsoObject, th: &IsoThumpable) -> bool {
        self.thumpable_hop_props(obj, th)
            .is_some_and(|p| p.has(self.t.TallHoppableN) || p.has(self.t.TallHoppableW))
    }

    /// `IsoThumpable.isThumpable()`.
    fn thumpable_thumpable(&self, sq: &IsoGridSquare, obj: &IsoObject, th: &IsoThumpable) -> bool {
        self.hooks.is_barricaded(sq, obj) || th.is_thumpable
    }

    /// `SquareUpdateTask.isWindowUnblocked(square, north)`.
    pub fn is_window_unblocked(&self, sq: &IsoGridSquare, north: bool) -> bool {
        for special in specials(sq) {
            match &special.kind {
                ObjectKind::Thumpable(th)
                    if self.is_window_object(special) && th.north == north =>
                {
                    return !self.hooks.is_barricaded(sq, special);
                }
                ObjectKind::Window(w) if w.north == north => {
                    if self.hooks.is_barricaded(sq, special) {
                        return false;
                    }
                    if self.window_invincible(sq, special, north) {
                        return false;
                    }
                    if w.open {
                        return true;
                    }
                    return w.destroyed && w.glass_removed;
                }
                _ => {}
            }
        }
        // `getWindowFrame(facing)`: the first `IsoWindowFrame` in `objects`
        // on that edge, then `canClimbThrough(null)`.
        let frame = sq
            .objects
            .iter()
            .find(|o| matches!(o.kind, ObjectKind::WindowFrame { north: n } if n == north));
        let Some(frame) = frame else {
            return false;
        };
        if let Some(i) = sq.window_index(north) {
            if self.hooks.is_barricaded(sq, &sq.objects[i]) {
                return false;
            }
        }
        !self.hooks.is_barricaded(sq, frame)
    }

    /// `IsoWindow.isInvincible()`.
    fn window_invincible(&self, sq: &IsoGridSquare, window: &IsoObject, north: bool) -> bool {
        let t = &self.t;
        if !sq.has(t.makeWindowInvincible) {
            return false;
        }
        if let Some(index) = sq.objects.iter().position(|o| o.id == window.id) {
            let cut = if north { t.cutN } else { t.cutW };
            for (i, o) in sq.objects.iter().enumerate() {
                if i == index {
                    continue;
                }
                if let Some(p) = props(self.sprites, o) {
                    if p.has(cut) && p.has(t.makeWindowInvincible) {
                        return true;
                    }
                }
            }
        }
        self.obj_has(window, t.makeWindowInvincible)
    }

    /// `SquareUpdateTask.hasSquareThumpable(square)`.
    fn has_square_thumpable(&self, sq: &IsoGridSquare) -> bool {
        if sq.has_stairs(self.sprites) {
            return false;
        }
        for obj in specials(sq) {
            if let ObjectKind::Thumpable(th) = &obj.kind {
                if self.thumpable_thumpable(sq, obj, th) && th.block_all_the_square {
                    return true;
                }
            }
        }
        sq.objects.iter().any(|o| o.moved_thumpable)
    }

    /// `SquareUpdateTask.hasWallThumpableN/W(square)`: decided by the first
    /// thumpable that is a wall-like obstacle.
    fn has_wall_thumpable(&self, sq: &IsoGridSquare, north: bool) -> bool {
        for obj in specials(sq) {
            let ObjectKind::Thumpable(th) = &obj.kind else {
                continue;
            };
            // `canClimbThrough(null)`: a window that is not barricaded.
            let climb_through = self.is_window_object(obj) && !self.hooks.is_barricaded(sq, obj);
            // `canClimbOver(null)`: hoppable.
            let climb_over = self.thumpable_hoppable(obj, th);
            if climb_through
                || climb_over && !self.thumpable_tall_hoppable(obj, th)
                || !self.thumpable_thumpable(sq, obj, th)
                || th.block_all_the_square
                || th.is_door
            {
                continue;
            }
            return (self.is_wall_object(obj, north) || th.is_corner) && !th.can_pass_through;
        }
        false
    }

    /// `SquareUpdateTask.hasWallBendableN/W(square)`.
    fn has_wall_bendable(&self, sq: &IsoGridSquare, north: bool) -> bool {
        sq.objects
            .iter()
            .any(|o| self.hooks.is_unbent_object(sq, o) && self.is_wall_object(o, north))
    }

    /// `IsoWindow.isSheetRopeHere(square)`.
    pub fn is_sheet_rope_here(&self, sq: Option<&IsoGridSquare>) -> bool {
        sq.is_some_and(|sq| self.t.climb_sheet.iter().any(|&f| sq.has(f)))
    }

    /// `IsoGridSquare.getBuilding() != null`: the square's room belongs to
    /// a building.
    pub fn has_building(&self, sq: &IsoGridSquare) -> bool {
        sq.room_def(self.meta).is_some_and(|r| r.building.is_some())
    }

    /// `HasSlopedRoof() && !HasEave()` (`HasSlopedRoofNorth` tests the west
    /// roof types too, as the original does).
    fn sloped_roof_without_eave(&self, sq: &IsoGridSquare) -> bool {
        let t = &self.t;
        let sloped =
            sq.has_type(t.WestRoofB) || sq.has_type(t.WestRoofM) || sq.has_type(t.WestRoofT);
        sloped && !sq.has(t.isEave)
    }

    /// `SquareUpdateTask.canClimbOverWall(square, dir)`.
    pub fn can_climb_over_wall(&self, sq: Option<&IsoGridSquare>, dir: IsoDirections) -> bool {
        let t = &self.t;
        let m = self.sprites;
        let Some(sq) = sq else { return false };
        let Some(adj) = sq.get_adjacent_square(dir, self.squares) else {
            return false;
        };
        if sq.have_roof || adj.have_roof {
            return false;
        }
        if !sq.treat_as_solid_floor(m) || !adj.treat_as_solid_floor(m) {
            return false;
        }
        if self.is_sheet_rope_here(Some(sq)) || self.is_sheet_rope_here(Some(adj)) {
            return false;
        }
        if self.has_building(sq) || self.has_building(adj) {
            return false;
        }
        if sq.has(t.water) || adj.has(t.water) {
            return false;
        }
        if sq.has(t.CantClimb) || adj.has(t.CantClimb) {
            return false;
        }
        if sq.has(t.solid) || sq.has(t.solidtrans) || adj.has(t.solid) || adj.has(t.solidtrans) {
            return false;
        }
        let above = self.squares.get_grid_square(sq.x, sq.y, sq.z + 1);
        if above.is_some_and(|a| self.sloped_roof_without_eave(a)) {
            return false;
        }
        let above2 = self.squares.get_grid_square(adj.x, adj.y, adj.z + 1);
        if above2.is_some_and(|a| self.sloped_roof_without_eave(a)) {
            return false;
        }
        if above.is_some_and(|a| a.has(t.collideN)) || above2.is_some_and(|a| a.has(t.collideN)) {
            return false;
        }
        !self.hooks.is_client() || !self.hooks.in_safehouse(sq) && !self.hooks.in_safehouse(adj)
    }
}

/// `getSlopedSurfaceHeightMin()`.
fn slope_min(sq: &IsoGridSquare) -> f32 {
    sq.surface.sloped_surface_height_min as f32 / 100.0
}

/// `getSlopedSurfaceHeightMax()`.
fn slope_max(sq: &IsoGridSquare) -> f32 {
    sq.surface.sloped_surface_height_max as f32 / 100.0
}

#[cfg(test)]
mod tests;
